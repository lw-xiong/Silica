//! What is drawn inside a card's frame.
//!
//! The view paints the frame (border, background, selection, the label
//! above a card and a group's title) itself and asks `Cards` for one element
//! per visible card, laid out with `layout_as_root` at the card's screen
//! size and clipped to the frame. `Cards` caches whatever is expensive to
//! build (a note's reading model, where an image lives) by node id, and
//! drops it when the node's content or the file it came from changes.
//!
//! A card's content is the reading view's: a text card's Markdown and a file
//! card's note go through the one parser (`reading::build_with`) and the one
//! block renderer (`reading::view::Renderer`), so a card shows exactly what
//! the reading view shows for the same text. Obsidian draws a card at a
//! fixed size and lets a CSS transform zoom it; gpui has no element
//! transform, so a card is instead rendered with every size multiplied by
//! the zoom: the font sizes through a scaled copy of the [`Theme`] and the
//! rem-based spacings through a scaled rem size ([`Scaled`]).

use crate::assets::icon;
use crate::explorer::display_name;
use crate::reading::model::{build_section, build_with, EmbedKind, Model, RBlock, Segment};
use crate::reading::view::{LinkHandler, ReadingHost, ReadingView, Renderer, ToggleHandler};
use crate::reading::{EmbedSource, LinkTarget, TaskMarker};
use crate::theme::Theme;
use crate::workspace::VaultState;
use gpui::{
    div, img, prelude::*, px, AnyElement, App, Bounds, Div, DivFrameState, Element, ElementId,
    FontWeight, GlobalElementId, Hitbox, InspectorElementId, LayoutId, ObjectFit, Pixels,
    SharedString, Size, WeakEntity, Window,
};
use silica_canvas::ZOOM_THRESHOLD;
use silica_formats::canvas::{BackgroundStyle, Node, NodeType};
use silica_formats::properties::PropertiesInDocument;
use silica_formats::FileKind;
use silica_markdown::{parse, parse_linktext, plain_text, Block, BlockKind, Inline, InlineKind};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

/// Space between a card's frame and its Markdown, in canvas units; Obsidian
/// pads its text and file cards by this much at zoom 1.
const CARD_PADDING: f32 = 16.0;
/// gpui's default rem size. The renderer's Tailwind-style spacings
/// (`px_3`, `pl_4`, `rounded_md`) are in rems, so scaling the rem size
/// scales them with the zoom.
const REM: f32 = 16.0;
/// The reading view's line height as a multiple of the font size.
const LINE_HEIGHT: f32 = 1.6;
/// A placeholder's icon, in canvas units.
const PLACEHOLDER_ICON: f32 = 40.0;
/// The zoomed-out title's font size as a fraction of the card's shorter
/// side, in canvas units, kept between the two limits: small enough to fit
/// a short card, large enough to read on a big one from far away.
const TITLE_RATIO: f32 = 0.2;
const TITLE_MIN: f32 = 20.0;
const TITLE_MAX: f32 = 120.0;
/// The cache holds at most this many cards' content before it drops the
/// ones not drawn lately. Each `render` call is one tick, so a card is
/// "lately drawn" when it was seen within the last `RECENT_TICKS` calls:
/// two full frames of a canvas with `MAX_ENTRIES` cards on screen.
const MAX_ENTRIES: usize = 512;
const RECENT_TICKS: u64 = 2 * MAX_ENTRIES as u64;

/// What a link inside a card does when clicked. The first argument is the
/// path the link is written in and resolves against: the canvas file for a
/// text card, the note for a file card.
pub type CardLinkHandler = Rc<dyn Fn(&str, LinkTarget, &mut Window, &mut App)>;

/// Where a clicked task checkbox lives: in a text node's `text` (the marker
/// is a line and column in that text) or in a note on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskOrigin {
    Node(String),
    File(String),
}

/// What a task checkbox inside a card does when clicked.
pub type CardTaskHandler = Rc<dyn Fn(&TaskOrigin, TaskMarker, &mut App)>;

/// The card contents, cached across frames.
#[derive(Default)]
pub struct Cards {
    /// The canvas file being shown; links in a text card resolve relative
    /// to it, as Obsidian's do.
    canvas_path: Option<String>,
    on_link: Option<CardLinkHandler>,
    on_task: Option<CardTaskHandler>,
    entries: HashMap<String, Entry>,
    /// Counts `render` calls; entries remember the tick they were last seen.
    tick: u64,
}

/// What a node's content was built from. A node whose key is unchanged
/// keeps its entry; a changed key (the text was edited, the file card points
/// elsewhere) rebuilds it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Key {
    Text(String),
    File {
        path: String,
        subpath: Option<String>,
    },
    Link(String),
    Group {
        background: Option<String>,
        style: Option<BackgroundStyle>,
    },
}

impl Key {
    fn of(node: &Node) -> Key {
        match node.kind {
            NodeType::Text => Key::Text(node.text.clone().unwrap_or_default()),
            NodeType::File => Key::File {
                path: node.file.clone().unwrap_or_default(),
                subpath: node.subpath.clone(),
            },
            NodeType::Link => Key::Link(node.url.clone().unwrap_or_default()),
            NodeType::Group => Key::Group {
                background: node.background.clone(),
                style: node.background_style,
            },
        }
    }
}

/// A vault file an entry was built from, so a change to it rebuilds the
/// entry. Paths are lowercased: vault comparisons are case-insensitive.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Dep {
    /// A vault-relative path: the card's own file, an embedded note.
    Path(String),
    /// Only a file name is known: an embedded image, which the model holds
    /// as an absolute path.
    Name(String),
}

impl Dep {
    fn path(path: &str) -> Dep {
        Dep::Path(path.to_lowercase())
    }

    fn name(path: &std::path::Path) -> Option<Dep> {
        path.file_name()
            .map(|n| Dep::Name(n.to_string_lossy().to_lowercase()))
    }

    /// Whether a change to the vault file `lower` (lowercased, its last
    /// segment in `name`) touches this dependency.
    fn matches(&self, lower: &str, name: &str) -> bool {
        match self {
            Dep::Path(p) => p == lower,
            Dep::Name(n) => n == name,
        }
    }
}

enum Content {
    /// Markdown, built by the one parser. `source` is the path links in it
    /// resolve against and `origin` where its task markers point.
    Note {
        model: Model,
        source: String,
        origin: TaskOrigin,
    },
    /// An image file card: the file, fitted inside the card.
    Image(PathBuf),
    /// A `.pdf` file card: its pages fitted to the card's width and
    /// scrolling inside it, opened at the node's `#page=`. The file is
    /// parsed by the document cache when the card is drawn, not here.
    Pdf {
        absolute: PathBuf,
        page: Option<usize>,
    },
    /// A file Silica does not draw inside a card: its name and kind.
    Placeholder {
        name: String,
        kind: FileKind,
    },
    /// Something the card points at is not there.
    Missing {
        name: String,
        /// Why, in the words shown beneath the name.
        reason: &'static str,
    },
    /// Obsidian embeds the page in a web view, which Silica has no way to
    /// draw; the card shows where the link goes instead.
    Link {
        host: String,
        url: String,
    },
    /// A group's background image and how it fills the group.
    Background {
        path: PathBuf,
        style: BackgroundStyle,
    },
    Empty,
}

struct Entry {
    key: Key,
    content: Content,
    /// What the card shows in place of its content when zoomed out.
    title: String,
    deps: Vec<Dep>,
    /// Callouts whose fold state the reader flipped, by title id; shared
    /// with the click handler so a flip survives the frame.
    toggled: Rc<RefCell<HashSet<usize>>>,
    last_seen: u64,
}

impl Cards {
    pub fn new() -> Cards {
        Cards::default()
    }

    /// The canvas file being shown. Call it when a canvas opens: links in
    /// a text card resolve relative to the canvas file, so the cache is
    /// emptied when the path changes.
    pub fn set_canvas_path(&mut self, path: Option<String>) {
        if self.canvas_path != path {
            self.canvas_path = path;
            self.clear();
        }
    }

    /// What a clicked link does. Unset, links are drawn but inert.
    pub fn set_on_link(&mut self, handler: CardLinkHandler) {
        self.on_link = Some(handler);
    }

    /// What a clicked task checkbox does. Unset, checkboxes are drawn but
    /// inert.
    pub fn set_on_task(&mut self, handler: CardTaskHandler) {
        self.on_task = Some(handler);
    }

    /// The element drawn inside `node`'s frame, filling `size` logical
    /// pixels. `scale` is screen pixels per canvas unit, so text can follow
    /// the zoom the way Obsidian's does (the card is a CSS transform there).
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        node: &Node,
        size: Size<Pixels>,
        scale: f32,
        state: &Arc<VaultState>,
        theme: &Theme,
        window: &mut Window,
        _cx: &mut App,
    ) -> AnyElement {
        self.tick += 1;
        let key = Key::of(node);
        let stale = self.entries.get(&node.id).is_none_or(|e| e.key != key);
        if stale {
            let (content, title, deps) = self.build(node, state);
            self.entries.insert(
                node.id.clone(),
                Entry {
                    key,
                    content,
                    title,
                    deps,
                    toggled: Rc::new(RefCell::new(HashSet::new())),
                    last_seen: self.tick,
                },
            );
            self.evict();
        }
        let tick = self.tick;
        if let Some(entry) = self.entries.get_mut(&node.id) {
            entry.last_seen = tick;
        }
        let entry = self
            .entries
            .get(&node.id)
            .expect("the entry was just inserted");

        // At and below the threshold Obsidian hides a card's content: a
        // text card shows a text icon, a note card its name; an image stays
        // an image and a group keeps its background (it is the group). Its
        // label is the view's. A link card keeps Silica's address card.
        // OBSERVE: whether a PDF card keeps its pages when zoomed out the
        // way an image does, or shows its name the way a note does; it
        // shows its name here.
        let zoomed_out = (scale as f64).log2() <= ZOOM_THRESHOLD;
        let keeps_content = matches!(entry.content, Content::Image(_))
            || matches!(node.kind, NodeType::Group | NodeType::Link);
        let body = if zoomed_out && !keeps_content {
            match node.kind {
                NodeType::Text => Cards::icon_placeholder("align-left", scale, theme),
                _ => Cards::title(&entry.title, size, scale, theme),
            }
        } else {
            match &entry.content {
                Content::Note {
                    model,
                    source,
                    origin,
                } => self.note(
                    model,
                    source,
                    origin,
                    &entry.toggled,
                    size,
                    scale,
                    state,
                    theme,
                    window,
                ),
                Content::Image(path) => img(path.clone())
                    .size_full()
                    .object_fit(ObjectFit::Contain)
                    .into_any_element(),
                // The same pages a note embed draws, filling the card: the
                // zoom reaches them through the card's width.
                Content::Pdf { absolute, page } => {
                    // "Open" in the card's toolbar opens the file, through
                    // the same link handler a card's links use.
                    let path = node.file.clone().unwrap_or_default();
                    let open: crate::pdf::embed::OpenPath = {
                        let handler = self.on_link.clone();
                        let source = path.clone();
                        Rc::new(move |target: &str, window: &mut Window, cx: &mut App| {
                            if let Some(handler) = &handler {
                                handler(&source, LinkTarget::Internal(target.into()), window, cx);
                            }
                        })
                    };
                    crate::pdf::embed::render_card(
                        absolute, &path, *page, &node.id, scale, theme, open,
                    )
                }
                Content::Placeholder { name, kind } => Cards::placeholder(
                    kind_icon(*kind),
                    name,
                    kind_label(*kind),
                    theme.text,
                    scale,
                    theme,
                ),
                Content::Missing { name, reason } => {
                    Cards::placeholder("file-search", name, reason, theme.text_muted, scale, theme)
                }
                Content::Link { host, url } => {
                    Cards::placeholder("link", host, url, theme.text, scale, theme)
                }
                Content::Background { path, style } => img(path.clone())
                    .size_full()
                    .object_fit(match style {
                        BackgroundStyle::Cover => ObjectFit::Cover,
                        BackgroundStyle::Ratio => ObjectFit::Contain,
                        // gpui draws an image once; tiling would need the
                        // image's own size, which it does not report before
                        // painting. Fitting it is the nearest it can do.
                        BackgroundStyle::Repeat => ObjectFit::Contain,
                    })
                    .into_any_element(),
                Content::Empty => div().into_any_element(),
            }
        };
        // The id scopes the element ids the renderer gives its runs of
        // text (they start at 0 in every model) to this one card.
        div()
            .id(ElementId::Name(format!("card-{}", node.id).into()))
            .w(size.width)
            .h(size.height)
            .overflow_hidden()
            .child(body)
            .into_any_element()
    }

    /// A vault file changed, was renamed or removed: forget what was built
    /// from it, including the cards that embed it.
    pub fn invalidate(&mut self, path: &str) {
        let lower = path.to_lowercase();
        let name = lower.rsplit('/').next().unwrap_or(&lower).to_string();
        self.entries
            .retain(|_, e| !e.deps.iter().any(|d| d.matches(&lower, &name)));
    }

    /// Another canvas is opening: forget everything.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Keeps the cache bounded: over the limit, the entries not drawn in
    /// the last couple of frames go; were every one of them drawn lately
    /// (a huge canvas, fully zoomed out), the half seen longest ago goes.
    fn evict(&mut self) {
        if self.entries.len() <= MAX_ENTRIES {
            return;
        }
        let tick = self.tick;
        self.entries
            .retain(|_, e| tick - e.last_seen <= RECENT_TICKS);
        if self.entries.len() > MAX_ENTRIES {
            let mut seen: Vec<u64> = self.entries.values().map(|e| e.last_seen).collect();
            seen.sort_unstable();
            let cutoff = seen[seen.len() / 2];
            self.entries.retain(|_, e| e.last_seen > cutoff);
        }
    }

    /// Builds a node's content, its zoomed-out title and the vault files
    /// it depends on.
    fn build(&self, node: &Node, state: &Arc<VaultState>) -> (Content, String, Vec<Dep>) {
        let vault: &dyn EmbedSource = state.as_ref();
        match node.kind {
            NodeType::Text => {
                let text = node.text.clone().unwrap_or_default();
                let source = self.canvas_path.clone().unwrap_or_default();
                let model = build_with(&source, &text, vault, state.build_options());
                let mut deps = Vec::new();
                collect_deps(&model.blocks, &mut deps);
                let title = first_line_title(&text);
                (
                    Content::Note {
                        model,
                        source,
                        origin: TaskOrigin::Node(node.id.clone()),
                    },
                    title,
                    deps,
                )
            }
            NodeType::File => {
                let path = node.file.clone().unwrap_or_default();
                let name = display_name(path.rsplit('/').next().unwrap_or(&path)).to_string();
                let mut deps = vec![Dep::path(&path)];
                let Some((path, kind)) = locate(state, &path) else {
                    return (
                        Content::Missing {
                            name: path,
                            reason: "Not in the vault",
                        },
                        name,
                        deps,
                    );
                };
                let content = match kind {
                    FileKind::Markdown => {
                        let Some(text) = vault.read(&path) else {
                            return (
                                Content::Missing {
                                    name: path,
                                    reason: "Not in the vault",
                                },
                                name,
                                deps,
                            );
                        };
                        let opts = state.build_options();
                        let model = match node.subpath.as_deref().map(subpath_linktext) {
                            None => Some(build_with(&path, &text, vault, opts)),
                            Some(linktext) => match parse_linktext(&linktext).subpath {
                                Some(sub) => build_section(&path, &text, &sub, vault, opts),
                                None => Some(build_with(&path, &text, vault, opts)),
                            },
                        };
                        match model {
                            Some(model) => {
                                collect_deps(&model.blocks, &mut deps);
                                Content::Note {
                                    model,
                                    source: path.clone(),
                                    origin: TaskOrigin::File(path),
                                }
                            }
                            None => Content::Missing {
                                name: format!("{name}{}", node.subpath.as_deref().unwrap_or("")),
                                reason: "No such heading or block",
                            },
                        }
                    }
                    FileKind::Image => Content::Image(vault.absolute(&path)),
                    // The node's `subpath` is the link's (`#page=2`), read
                    // the way an embed's is.
                    FileKind::Pdf => Content::Pdf {
                        absolute: vault.absolute(&path),
                        page: node
                            .subpath
                            .as_deref()
                            .and_then(|s| silica_pdf::parse_subpath(s).page),
                    },
                    kind => Content::Placeholder {
                        name: path.rsplit('/').next().unwrap_or(&path).to_string(),
                        kind,
                    },
                };
                (content, name, deps)
            }
            NodeType::Link => {
                let url = node.url.clone().unwrap_or_default();
                let mut host = host_of(&url);
                if host.is_empty() {
                    host = url.clone();
                }
                let title = host.clone();
                (Content::Link { host, url }, title, Vec::new())
            }
            NodeType::Group => {
                let Some(background) = node.background.as_deref().filter(|b| !b.is_empty()) else {
                    return (Content::Empty, String::new(), Vec::new());
                };
                let deps = vec![Dep::path(background)];
                let content = match locate(state, background) {
                    Some((path, _)) => Content::Background {
                        path: vault.absolute(&path),
                        // Unset means `cover`: Obsidian writes the key only
                        // when the user picked another style.
                        style: node.background_style.unwrap_or(BackgroundStyle::Cover),
                    },
                    None => Content::Empty,
                };
                (content, String::new(), deps)
            }
        }
    }

    /// A note's blocks, rendered as the reading view renders them, at the
    /// zoom.
    #[allow(clippy::too_many_arguments)]
    fn note(
        &self,
        model: &Model,
        source: &str,
        origin: &TaskOrigin,
        toggled: &Rc<RefCell<HashSet<usize>>>,
        size: Size<Pixels>,
        scale: f32,
        state: &Arc<VaultState>,
        theme: &Theme,
        window: &mut Window,
    ) -> AnyElement {
        // Every font size in the renderer is a multiple of the base, so a
        // scaled base scales the text; the rem size scales the spacing.
        let mut scaled = theme.clone();
        scaled.base_font_size = theme.base_font_size * scale;
        let base = scaled.base_font_size;
        let pad = CARD_PADDING * scale;
        let available = f32::from(size.width) - 2. * pad;
        let flipped = toggled.borrow().clone();
        let types = state.property_types();
        let properties =
            PropertiesInDocument::from_config(state.app().properties_in_document.as_deref());
        let on_link: LinkHandler = {
            let handler = self.on_link.clone();
            let source = source.to_string();
            Rc::new(move |target, window, cx| {
                if let Some(handler) = &handler {
                    handler(&source, target, window, cx);
                }
            })
        };
        let on_toggle: ToggleHandler = {
            let toggled = toggled.clone();
            Rc::new(move |id, cx| {
                let mut toggled = toggled.borrow_mut();
                if !toggled.remove(&id) {
                    toggled.insert(id);
                }
                // The handler has no entity to notify; the window redraws
                // and the next `render` reads the new fold state.
                cx.refresh_windows();
            })
        };
        let host = ReadingHost {
            properties,
            // The copy button copies; its check does not flash in a card.
            copied: None,
            on_copy: Rc::new(|_, _| {}),
            on_task: {
                let handler = self.on_task.clone();
                let origin = origin.clone();
                Rc::new(move |marker, cx| {
                    if let Some(handler) = &handler {
                        handler(&origin, marker, cx);
                    }
                })
            },
            preview_resolver: None,
        };
        let renderer = Renderer::new(
            &scaled,
            window.text_system().clone(),
            available,
            &flipped,
            &types,
            // Read-only, like the reading view.
            None,
            None,
            on_link,
            on_toggle,
            String::new(),
            WeakEntity::<ReadingView>::new_invalid(),
        )
        .with_reading_host(&host);
        let blocks: Vec<AnyElement> = model
            .blocks
            .iter()
            .map(|b| renderer.block(b, false))
            .collect();
        Scaled::new(
            px(REM * scale),
            div()
                .w(size.width)
                .p(px(pad))
                .font_family(theme.text_font.clone())
                .text_size(px(base))
                .line_height(px(base * LINE_HEIGHT))
                .text_color(theme.text)
                .flex()
                .flex_col()
                .children(blocks),
        )
        .into_any_element()
    }

    /// An icon over a name over a muted line, centered in the card.
    /// The zoomed-out stand-in of a text card: Obsidian's `align-left`
    /// icon, centred.
    fn icon_placeholder(name: &'static str, scale: f32, theme: &Theme) -> AnyElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(icon(name, PLACEHOLDER_ICON * scale, theme.text))
            .into_any_element()
    }

    fn placeholder(
        icon_name: &str,
        name: &str,
        detail: &str,
        name_color: gpui::Hsla,
        scale: f32,
        theme: &Theme,
    ) -> AnyElement {
        let base = theme.base_font_size * scale;
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(base * 0.5))
            .px(px(CARD_PADDING * scale))
            .font_family(theme.ui_font.clone())
            .child(icon(icon_name, PLACEHOLDER_ICON * scale, theme.text_muted))
            .child(
                div()
                    .max_w_full()
                    .truncate()
                    .text_size(px(base * 1.1))
                    .line_height(px(base * 1.1 * 1.3))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(name_color)
                    .child(SharedString::from(name.to_string())),
            )
            .child(
                div()
                    .max_w_full()
                    .truncate()
                    .text_size(px(base * 0.85))
                    .line_height(px(base * 0.85 * 1.3))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(detail.to_string())),
            )
            .into_any_element()
    }

    /// The large single-line title a card shows below the zoom threshold.
    fn title(text: &str, size: Size<Pixels>, scale: f32, theme: &Theme) -> AnyElement {
        let short = f32::from(size.width).min(f32::from(size.height)) / scale;
        let font = (short * TITLE_RATIO).clamp(TITLE_MIN, TITLE_MAX) * scale;
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .px(px(CARD_PADDING * scale))
            .font_family(theme.text_font.clone())
            .child(
                div()
                    .max_w_full()
                    .truncate()
                    .text_size(px(font))
                    .line_height(px(font * 1.2))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text)
                    .child(SharedString::from(text.to_string())),
            )
            .into_any_element()
    }
}

/// Finds `path` in the vault, case-insensitively, giving back the path as
/// the vault spells it and the file's kind.
fn locate(state: &VaultState, path: &str) -> Option<(String, FileKind)> {
    let vault = state.vault();
    if let Some(file) = vault
        .files()
        .iter()
        .find(|f| f.path.eq_ignore_ascii_case(path))
    {
        return Some((file.path.clone(), file.kind));
    }
    // Not in the scan (a file that just appeared, a folder the scan skips):
    // the disk has the last word.
    if vault.exists(path) {
        let ext = path.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
        return Some((path.to_string(), FileKind::from_extension(ext)));
    }
    None
}

/// A node's `subpath` as the subpath part of a link body, so the link
/// parser reads it. Obsidian writes it with its `#`; one written without is
/// taken as a heading all the same.
fn subpath_linktext(subpath: &str) -> String {
    if subpath.starts_with('#') {
        subpath.to_string()
    } else {
        format!("#{subpath}")
    }
}

/// The Lucide icon for a file kind's placeholder. Only the icons the shell
/// already bundles are available here; the ones Obsidian uses for these
/// kinds (`file-text`, `file-audio`, `file-video`, `file`) and `globe` for
/// a link card are one line each in `assets.rs` once they are fetched.
fn kind_icon(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Canvas => "layout-dashboard",
        FileKind::Base => "layout-list",
        FileKind::Pdf => "book-open",
        _ => "files",
    }
}

/// What a placeholder says a file is.
fn kind_label(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Markdown => "Note",
        FileKind::Canvas => "Canvas",
        FileKind::Base => "Base",
        FileKind::Image => "Image",
        FileKind::Audio => "Audio",
        FileKind::Video => "Video",
        FileKind::Pdf => "PDF",
        FileKind::Other => "File",
    }
}

/// The vault files a built model shows besides its own: embedded notes and
/// PDFs by path and embedded images by file name.
fn collect_deps(blocks: &[RBlock], out: &mut Vec<Dep>) {
    for block in blocks {
        match block {
            RBlock::Paragraph(segments) => {
                for segment in segments {
                    if let Segment::Embed(embed) = segment {
                        match &embed.kind {
                            EmbedKind::Note { path, blocks, .. } => {
                                out.push(Dep::path(path));
                                collect_deps(blocks, out);
                            }
                            EmbedKind::Image(path) => out.extend(Dep::name(path)),
                            EmbedKind::Pdf(pdf) => out.push(Dep::path(&pdf.path)),
                            _ => {}
                        }
                    }
                }
            }
            RBlock::Quote(children) | RBlock::Callout { children, .. } => {
                collect_deps(children, out)
            }
            RBlock::List { items, .. } => {
                for item in items {
                    collect_deps(&item.children, out);
                }
            }
            RBlock::Footnotes(notes) => {
                for (_, body) in notes {
                    collect_deps(body, out);
                }
            }
            _ => {}
        }
    }
}

/// The first line of `text` with its Markdown stripped: what a text card
/// shows in place of its content when zoomed out. The parser finds the
/// first block that has any text (a properties block, a rule or a comment
/// is skipped) and its inline content is read up to the first line break.
pub(crate) fn first_line_title(text: &str) -> String {
    let doc = parse(text);
    first_text(&doc.blocks)
        .map(|t| t.trim().to_string())
        .unwrap_or_default()
}

fn first_text(blocks: &[Block<'_>]) -> Option<String> {
    for block in blocks {
        let found = match &block.kind {
            BlockKind::Yaml { .. } | BlockKind::ThematicBreak | BlockKind::Comment { .. } => None,
            BlockKind::Paragraph { content } | BlockKind::Heading { content, .. } => {
                first_inline_line(content)
            }
            BlockKind::CodeBlock { content, .. }
            | BlockKind::HtmlBlock { content }
            | BlockKind::Math { content } => content
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .map(str::to_string),
            BlockKind::BlockQuote { children } | BlockKind::FootnoteDefinition { children, .. } => {
                first_text(children)
            }
            BlockKind::Callout {
                title, children, ..
            } => silica_markdown::inline_runs(title)
                .first()
                .and_then(|run| first_inline_line(run))
                .or_else(|| first_text(children)),
            BlockKind::List { items, .. } => {
                items.iter().find_map(|item| first_text(&item.children))
            }
            BlockKind::Table { header, .. } => {
                let cells: Vec<String> = header
                    .iter()
                    .map(|c| plain_text(&c.content).trim().to_string())
                    .filter(|c| !c.is_empty())
                    .collect();
                (!cells.is_empty()).then(|| cells.join(" "))
            }
        };
        if let Some(text) = found.filter(|t| !t.trim().is_empty()) {
            return Some(text);
        }
    }
    None
}

/// The plain text of `content` up to its first line break.
fn first_inline_line(content: &[Inline<'_>]) -> Option<String> {
    let end = content
        .iter()
        .position(|i| matches!(i.kind, InlineKind::SoftBreak | InlineKind::HardBreak))
        .unwrap_or(content.len());
    let text = plain_text(&content[..end]);
    (!text.trim().is_empty()).then_some(text)
}

/// The host of a URL, lowercased, without user info or port; empty when
/// the text has no authority (`mailto:`, a bare word).
pub(crate) fn host_of(url: &str) -> String {
    let url = url.trim();
    let Some((_, rest)) = url.split_once("://") else {
        return String::new();
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let host = if host.starts_with('[') {
        // An IPv6 literal keeps its brackets and loses only a port after them.
        match host.find(']') {
            Some(end) => &host[..=end],
            None => host,
        }
    } else {
        host.split(':').next().unwrap_or(host)
    };
    host.to_ascii_lowercase()
}

/// Sets gpui's rem size for its child so the renderer's rem-based spacings
/// follow the zoom along with the font sizes. The same thing Zed's
/// `ui::WithRemSize` does; gpui has no element transform, and the rem
/// size is read while a style is turned into a layout, so every phase runs
/// under the override.
struct Scaled {
    div: Div,
    rem_size: Pixels,
}

impl Scaled {
    fn new(rem_size: Pixels, child: Div) -> Scaled {
        Scaled {
            div: div().child(child),
            rem_size,
        }
    }
}

impl IntoElement for Scaled {
    type Element = Scaled;

    fn into_element(self) -> Scaled {
        self
    }
}

impl Element for Scaled {
    type RequestLayoutState = DivFrameState;
    type PrepaintState = Option<Hitbox>;

    fn id(&self) -> Option<ElementId> {
        Element::id(&self.div)
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        Element::source_location(&self.div)
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, DivFrameState) {
        window.with_rem_size(Some(self.rem_size), |window| {
            self.div.request_layout(id, inspector_id, window, cx)
        })
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut DivFrameState,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Hitbox> {
        window.with_rem_size(Some(self.rem_size), |window| {
            self.div
                .prepaint(id, inspector_id, bounds, request_layout, window, cx)
        })
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut DivFrameState,
        prepaint: &mut Option<Hitbox>,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_rem_size(Some(self.rem_size), |window| {
            self.div.paint(
                id,
                inspector_id,
                bounds,
                request_layout,
                prepaint,
                window,
                cx,
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reading::BuildOptions;

    /// A vault of in-memory notes for building sections without a disk.
    struct Stub(Vec<(&'static str, &'static str)>);

    impl EmbedSource for Stub {
        fn resolve(&self, linktext: &str, _source: &str) -> Option<String> {
            let target = parse_linktext(linktext).path.trim_end_matches(".md");
            // By file name, the way a shortest-path vault resolves.
            self.0
                .iter()
                .map(|(p, _)| *p)
                .find(|p| {
                    let name = p.rsplit('/').next().unwrap_or(p).trim_end_matches(".md");
                    name.eq_ignore_ascii_case(target)
                })
                .map(str::to_string)
        }
        fn read(&self, path: &str) -> Option<String> {
            self.0
                .iter()
                .find(|(p, _)| *p == path)
                .map(|(_, t)| t.to_string())
        }
        fn absolute(&self, path: &str) -> PathBuf {
            PathBuf::from("/vault").join(path)
        }
    }

    fn entry(key: Key, deps: Vec<Dep>, last_seen: u64) -> Entry {
        Entry {
            key,
            content: Content::Empty,
            title: String::new(),
            deps,
            toggled: Rc::new(RefCell::new(HashSet::new())),
            last_seen,
        }
    }

    fn headings(model: &Model) -> Vec<String> {
        model
            .blocks
            .iter()
            .filter_map(|b| match b {
                RBlock::Heading { plain, .. } => Some(plain.clone()),
                _ => None,
            })
            .collect()
    }

    const NOTE: &str = "---\ntags: [a]\n---\n# Top\n\nIntro ^intro\n\n## One\n\nFirst body\n\n### Deeper\n\nStill one\n\n## Two\n\nSecond body\n";

    #[test]
    fn a_heading_section_runs_to_the_next_heading_of_its_level() {
        let stub = Stub(vec![("Note.md", NOTE)]);
        let sub = parse_linktext("#One").subpath.unwrap();
        let model = build_section("Note.md", NOTE, &sub, &stub, BuildOptions::default()).unwrap();
        assert_eq!(headings(&model), ["One", "Deeper"]);
        // Four blocks: the heading, a paragraph, the subheading, a paragraph.
        assert_eq!(model.blocks.len(), 4);
        // The starts are positions in the note, so the view can scroll by them.
        assert_eq!(model.starts[0], NOTE.find("## One").unwrap());
    }

    #[test]
    fn a_nested_heading_path_and_a_block_id_select_one_part() {
        let stub = Stub(vec![("Note.md", NOTE)]);
        let sub = parse_linktext("#One#Deeper").subpath.unwrap();
        let model = build_section("Note.md", NOTE, &sub, &stub, BuildOptions::default()).unwrap();
        assert_eq!(headings(&model), ["Deeper"]);
        assert_eq!(model.blocks.len(), 2);

        let sub = parse_linktext("#^intro").subpath.unwrap();
        let model = build_section("Note.md", NOTE, &sub, &stub, BuildOptions::default()).unwrap();
        assert_eq!(model.blocks.len(), 1);
        assert!(matches!(model.blocks[0], RBlock::Paragraph(_)));

        let sub = parse_linktext("#Nowhere").subpath.unwrap();
        assert!(build_section("Note.md", NOTE, &sub, &stub, BuildOptions::default()).is_none());
    }

    #[test]
    fn a_subpath_written_without_its_hash_still_parses() {
        assert_eq!(subpath_linktext("#One"), "#One");
        assert_eq!(subpath_linktext("One"), "#One");
        assert!(matches!(
            parse_linktext(&subpath_linktext("^abc")).subpath,
            Some(silica_markdown::Subpath::Block("abc"))
        ));
    }

    #[test]
    fn the_title_is_the_first_line_with_markdown_stripped() {
        assert_eq!(
            first_line_title("# A **bold** heading\n\nBody"),
            "A bold heading"
        );
        assert_eq!(first_line_title("First line\nsecond line"), "First line");
        assert_eq!(first_line_title("First line  \nsecond line"), "First line");
        assert_eq!(
            first_line_title("- [ ] a task with [[Link|alias]]"),
            "a task with alias"
        );
        assert_eq!(first_line_title("> [!note] Titled\n> body"), "Titled");
        assert_eq!(first_line_title("> [!note]\n> body"), "body");
        assert_eq!(
            first_line_title("---\ntitle: x\n---\n\n---\n\nAfter"),
            "After"
        );
        assert_eq!(first_line_title("```rust\nlet x = 1;\n```"), "let x = 1;");
        assert_eq!(first_line_title("| a | b |\n|---|---|\n| 1 | 2 |"), "a b");
        assert_eq!(first_line_title("%% hidden %%\n\nShown"), "Shown");
        assert_eq!(first_line_title(""), "");
        assert_eq!(first_line_title("   \n\n"), "");
    }

    #[test]
    fn the_host_drops_scheme_path_user_and_port() {
        assert_eq!(host_of("https://Obsidian.md/canvas"), "obsidian.md");
        assert_eq!(
            host_of("http://user:pw@example.com:8080/x?y#z"),
            "example.com"
        );
        assert_eq!(host_of("  https://a.b.c  "), "a.b.c");
        assert_eq!(host_of("http://[::1]:8080/"), "[::1]");
        assert_eq!(host_of("mailto:someone@example.com"), "");
        assert_eq!(host_of("not a url"), "");
    }

    #[test]
    fn the_key_follows_the_fields_a_kind_uses() {
        let mut node = Node::new("a", NodeType::Text, 0., 0., 100., 100.);
        node.text = Some("x".into());
        node.file = Some("ignored.md".into());
        assert_eq!(Key::of(&node), Key::Text("x".into()));
        node.kind = NodeType::File;
        node.subpath = Some("#H".into());
        assert_eq!(
            Key::of(&node),
            Key::File {
                path: "ignored.md".into(),
                subpath: Some("#H".into()),
            }
        );
    }

    #[test]
    fn invalidation_is_by_path_or_by_embedded_file_name_and_ignores_case() {
        let mut cards = Cards::new();
        cards.entries.insert(
            "note".into(),
            entry(
                Key::Text(String::new()),
                vec![Dep::path("Folder/Note.md")],
                0,
            ),
        );
        cards.entries.insert(
            "embeds".into(),
            entry(
                Key::Text(String::new()),
                vec![
                    Dep::path("Other.md"),
                    Dep::name(std::path::Path::new("/vault/img/Pic.PNG")).unwrap(),
                ],
                0,
            ),
        );
        cards.entries.insert(
            "link".into(),
            entry(Key::Link("https://x".into()), vec![], 0),
        );
        cards.invalidate("folder/note.md");
        assert!(!cards.entries.contains_key("note"));
        assert!(cards.entries.contains_key("embeds"));
        cards.invalidate("Attachments/pic.png");
        assert!(!cards.entries.contains_key("embeds"));
        assert!(cards.entries.contains_key("link"));
        cards.clear();
        assert!(cards.entries.is_empty());
    }

    #[test]
    fn eviction_keeps_the_recently_drawn_and_stays_bounded() {
        let mut cards = Cards::new();
        cards.tick = 10_000;
        for i in 0..MAX_ENTRIES + 10 {
            // The first ten were last drawn long ago, the rest just now.
            let seen = if i < 10 { 0 } else { cards.tick - i as u64 };
            cards.entries.insert(
                format!("n{i}"),
                entry(Key::Link(String::new()), vec![], seen),
            );
        }
        cards.evict();
        assert_eq!(cards.entries.len(), MAX_ENTRIES);
        assert!(!cards.entries.contains_key("n0"));
        assert!(cards.entries.contains_key("n10"));

        // Every entry drawn this frame: the half seen longest ago goes.
        let mut cards = Cards::new();
        cards.tick = 2 * MAX_ENTRIES as u64;
        for i in 0..2 * MAX_ENTRIES {
            cards.entries.insert(
                format!("n{i}"),
                entry(Key::Link(String::new()), vec![], i as u64),
            );
        }
        cards.evict();
        assert!(cards.entries.len() <= MAX_ENTRIES);
        assert!(cards
            .entries
            .contains_key(&format!("n{}", 2 * MAX_ENTRIES - 1)));
        assert!(!cards.entries.contains_key("n0"));
    }

    #[test]
    fn a_models_dependencies_are_its_embeds() {
        let stub = Stub(vec![
            (
                "Main.md",
                "![[Other]]\n\n> ![[Pic.png]]\n\n- ![[Third#H]]\n",
            ),
            ("Other.md", "other\n"),
            ("Third.md", "# H\n\nthird\n"),
            ("img/Pic.png", ""),
        ]);
        let model = build_with(
            "Main.md",
            "![[Other]]\n\n> ![[Pic.png]]\n\n- ![[Third#H]]\n",
            &stub,
            BuildOptions::default(),
        );
        let mut deps = Vec::new();
        collect_deps(&model.blocks, &mut deps);
        assert!(deps.contains(&Dep::path("Other.md")), "{deps:?}");
        assert!(deps.contains(&Dep::path("Third.md")), "{deps:?}");
        assert!(deps.contains(&Dep::Name("pic.png".into())), "{deps:?}");
    }

    #[test]
    fn an_embedded_pdf_is_a_dependency_by_path() {
        let stub = Stub(vec![
            ("Main.md", "![[Field.pdf#page=2]]\n"),
            ("docs/Field.pdf", ""),
        ]);
        let model = build_with(
            "Main.md",
            "![[Field.pdf#page=2]]\n",
            &stub,
            BuildOptions::default(),
        );
        let mut deps = Vec::new();
        collect_deps(&model.blocks, &mut deps);
        assert!(deps.contains(&Dep::path("docs/Field.pdf")), "{deps:?}");
    }

    /// A `.pdf` file node is a PDF card with the node's `#page=`, and the
    /// file is listed as its dependency; the card reads nothing of it (the
    /// file here is empty and would not parse).
    #[test]
    fn a_pdf_file_node_is_a_pdf_card_opened_at_its_page() {
        use std::fs;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join(".obsidian")).unwrap();
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::write(root.join("docs/Field.pdf"), "").unwrap();
        let state = Arc::new(VaultState::load(&root.to_path_buf()).unwrap());
        let cards = Cards::new();

        let mut node = Node::new("n", NodeType::File, 0., 0., 400., 300.);
        node.file = Some("docs/Field.pdf".into());
        node.subpath = Some("#page=2".into());
        let (content, title, deps) = cards.build(&node, &state);
        match content {
            Content::Pdf { absolute, page } => {
                assert_eq!(absolute, root.join("docs/Field.pdf"));
                assert_eq!(page, Some(1));
            }
            _ => panic!("not a PDF card"),
        }
        assert_eq!(title, "Field.pdf");
        assert!(deps.contains(&Dep::path("docs/Field.pdf")), "{deps:?}");

        // Without a subpath the card opens at the first page.
        node.subpath = None;
        let (content, _, _) = cards.build(&node, &state);
        assert!(matches!(content, Content::Pdf { page: None, .. }));
    }
}
