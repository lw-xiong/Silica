//! Source-mode editor: a [`TextBuffer`] soft-wrapped at the view width,
//! with the decorations of the shared parser.
//!
//! The text itself, its line index, the selections, the undo history and
//! every edit the keys below make live in `silica-text`, where they can be
//! tested without a window; this view adds the shaping, scrolling,
//! decorations, find panel and properties table around them.
//!
//! Only the lines on screen are shaped and painted each frame. The scroll
//! position is a line plus a pixel offset into it rather than an absolute
//! pixel, so wrapped-row heights are only ever measured for lines near the
//! viewport and a 50k-line note never needs a full layout pass. Decorations
//! come from `silica_markdown::highlight`, recomputed on a background thread
//! after every edit and swapped in when they arrive, so typing never waits
//! for a parse. Text input goes through GPUI's input handler (so IME and
//! dead keys work) scoped to the cursor's line; everything else is an action
//! bound in [`bind_keys`].
//!
//! The buffer holds `\n` line endings; the file's own ending and BOM are
//! remembered at load and restored by [`Editor::file_contents`], so a save
//! changes nothing but the edited bytes (the round trips in
//! `silica-text/tests/writer_roundtrip.rs` hold that over the corpus).

use crate::fold::{self, Foldable};
use crate::i18n::t;
use crate::math::SpaceWidths;
use crate::preview::{self, DisplayLine, Piece};
use crate::properties::Field;
use crate::reading::model::{
    build_widgets, BuildOptions, EmbedSource, InlineStyle, LinkTarget, RBlock, Widget,
};
use crate::reading::view::{LinkHandler, PropertiesHost, ReadingView, Renderer, ToggleHandler};
use crate::reading::view::{TableAction, TableHost};

use crate::suggest::SuggestState;
use crate::text_input::{InputEvent, TextInput};
use crate::theme::Theme;
use crate::workspace::VaultState;
use gpui::{
    actions, div, fill, point, prelude::*, px, relative, size, AnyElement, App, AvailableSpace,
    Bounds, ClipboardItem, Context, CursorStyle, DispatchPhase, Element, ElementId,
    ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable, Font,
    FontStyle, FontWeight, GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId,
    KeyBinding, KeyDownEvent, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PaintQuad, PathBuilder, Pixels, Point, ScrollWheelEvent, SharedString, StrikethroughStyle,
    Style, Subscription, Task, TextAlign, TextRun, TextStyle, UTF16Selection, UnderlineStyle,
    WeakEntity, Window, WrappedLine,
};
use silica_formats::config::PropertyTypes;
use silica_formats::properties::{self as props, PropertiesInDocument};
use silica_markdown::highlight::{flatten, highlight_lines, HighlightKind, HighlightSpan};
use silica_markdown::{Block, BlockKind, Document};
use silica_suggest::{self as suggest};
use silica_text::table::{self, TableCommand};
use silica_text::{
    autopair, find, wrap, BlockInsert, Cursor, EditKind, LinePrefix, LinkSyntax, Sel, SpellChecker,
    TextBuffer,
};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

mod actions;
pub(crate) mod attach;
mod folding;
mod mouse;
mod movement;
mod scrolling;
mod search;

actions!(
    editor,
    [
        MoveLeft,
        MoveRight,
        MoveUp,
        MoveDown,
        MoveWordLeft,
        MoveWordRight,
        LineStart,
        LineEnd,
        PageUp,
        PageDown,
        DocumentStart,
        DocumentEnd,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectWordLeft,
        SelectWordRight,
        SelectLineStart,
        SelectLineEnd,
        SelectPageUp,
        SelectPageDown,
        SelectDocumentStart,
        SelectDocumentEnd,
        SelectAll,
        Backspace,
        Delete,
        DeleteWordLeft,
        DeleteWordRight,
        Newline,
        NewlinePlain,
        Tab,
        Outdent,
        Undo,
        Redo,
        Copy,
        Cut,
        Paste,
        FindNext,
        FindPrevious,
        FindAll,
        CloseFind,
        AddCursorAbove,
        AddCursorBelow,
        FocusOtherField,
    ]
);

/// Binds the editor's navigation and editing keys; call once at startup.
pub fn bind_keys(cx: &mut App) {
    let ctx = Some("Editor");
    cx.bind_keys([
        KeyBinding::new("left", MoveLeft, ctx),
        KeyBinding::new("right", MoveRight, ctx),
        KeyBinding::new("up", MoveUp, ctx),
        KeyBinding::new("down", MoveDown, ctx),
        KeyBinding::new("ctrl-left", MoveWordLeft, ctx),
        KeyBinding::new("ctrl-right", MoveWordRight, ctx),
        KeyBinding::new("home", LineStart, ctx),
        KeyBinding::new("end", LineEnd, ctx),
        KeyBinding::new("pageup", PageUp, ctx),
        KeyBinding::new("pagedown", PageDown, ctx),
        KeyBinding::new("ctrl-home", DocumentStart, ctx),
        KeyBinding::new("ctrl-end", DocumentEnd, ctx),
        KeyBinding::new("shift-left", SelectLeft, ctx),
        KeyBinding::new("shift-right", SelectRight, ctx),
        KeyBinding::new("shift-up", SelectUp, ctx),
        KeyBinding::new("shift-down", SelectDown, ctx),
        KeyBinding::new("ctrl-shift-left", SelectWordLeft, ctx),
        KeyBinding::new("ctrl-shift-right", SelectWordRight, ctx),
        KeyBinding::new("shift-home", SelectLineStart, ctx),
        KeyBinding::new("shift-end", SelectLineEnd, ctx),
        KeyBinding::new("shift-pageup", SelectPageUp, ctx),
        KeyBinding::new("shift-pagedown", SelectPageDown, ctx),
        KeyBinding::new("ctrl-shift-home", SelectDocumentStart, ctx),
        KeyBinding::new("ctrl-shift-end", SelectDocumentEnd, ctx),
        KeyBinding::new("ctrl-a", SelectAll, ctx),
        KeyBinding::new("backspace", Backspace, ctx),
        KeyBinding::new("shift-backspace", Backspace, ctx),
        KeyBinding::new("delete", Delete, ctx),
        KeyBinding::new("ctrl-backspace", DeleteWordLeft, ctx),
        KeyBinding::new("ctrl-delete", DeleteWordRight, ctx),
        KeyBinding::new("enter", Newline, ctx),
        KeyBinding::new("shift-enter", NewlinePlain, ctx),
        KeyBinding::new("tab", Tab, ctx),
        KeyBinding::new("shift-tab", Outdent, ctx),
        KeyBinding::new("ctrl-z", Undo, ctx),
        KeyBinding::new("ctrl-shift-z", Redo, ctx),
        KeyBinding::new("ctrl-y", Redo, ctx),
        KeyBinding::new("ctrl-c", Copy, ctx),
        KeyBinding::new("ctrl-x", Cut, ctx),
        KeyBinding::new("ctrl-v", Paste, ctx),
        KeyBinding::new("escape", CloseFind, ctx),
        KeyBinding::new("shift-enter", FindPrevious, Some("FindPanel")),
        KeyBinding::new("alt-enter", FindAll, Some("FindPanel")),
        KeyBinding::new("escape", CloseFind, Some("FindPanel")),
        KeyBinding::new("tab", FocusOtherField, Some("FindPanel")),
        KeyBinding::new("shift-tab", FocusOtherField, Some("FindPanel")),
        // * The popup's own keys, in the context the editor only carries
        //   while it is open, so Ctrl+N still makes a new note otherwise.
        KeyBinding::new("ctrl-n", MoveDown, Some("SuggestPopup")),
        KeyBinding::new("ctrl-p", MoveUp, Some("SuggestPopup")),
    ]);
}

/// The dominant line ending of `text`, which is what a write into it uses.
fn newline_of(text: &str) -> &'static str {
    let crlf = text.matches("\r\n").count();
    if crlf > 0 && crlf * 2 >= text.matches('\n').count() {
        "\r\n"
    } else {
        "\n"
    }
}

/// Horizontal padding either side of the text.
const GUTTER: f32 = 24.0;
/// Obsidian's readable line length (`--file-line-width`).
const READABLE_WIDTH: f32 = 700.0;
/// Width of the vertical scrollbar's track at the right edge.
const SCROLLBAR_WIDTH: f32 = 12.0;
/// The thumb never shrinks below this, so it can still be grabbed.
const SCROLLBAR_MIN_THUMB: f32 = 24.0;

#[derive(Clone)]
pub enum EditorEvent {
    /// The buffer changed; the owner schedules a save.
    Edited,
    /// A rendered link was clicked in Live Preview.
    OpenLink(LinkTarget),
    /// A link was Ctrl+clicked, in either editing mode: it opens in a new
    /// tab, as Obsidian's Mod+click does.
    OpenLinkInNewTab(LinkTarget),
    /// Something the editor could not do, for the status bar.
    Reported(SharedString),
    /// A property's type was chosen from its menu; the workspace records it
    /// in the vault's `types.json`.
    PropertyTypeAssigned {
        key: String,
        ty: props::PropertyType,
    },
    /// Files in the vault changed (e.g. an attachment was saved); the workspace absorbs them.
    FilesChanged(crate::workspace::Changes),
    /// Context menu: cut the selection.
    Cut,
    /// Context menu: copy the selection.
    Copy,
    /// Context menu: paste from the clipboard.
    Paste,
    /// Context menu: select all text.
    SelectAll,
    /// Context menu: toggle bold on the selection.
    ToggleBold,
    /// Context menu: toggle italic on the selection.
    ToggleItalic,
    /// Context menu: toggle highlight on the selection.
    ToggleHighlight,
    /// Context menu: insert a wikilink at the caret.
    InsertWikilink,
    /// Context menu: insert a markdown link at the caret.
    InsertLink,
    /// Context menu: insert an embed at the caret.
    InsertEmbed,
    /// Context menu: toggle a checkbox on the current line.
    ToggleCheckbox,
    /// Context menu: set heading level 1.
    SetHeading1,
    /// Context menu: set heading level 2.
    SetHeading2,
    /// Context menu: toggle bullet list on the current line.
    ToggleBullet,
    /// Context menu: toggle numbered list on the current line.
    ToggleNumbered,
    /// Context menu: toggle blockquote on the current line.
    ToggleBlockquote,
}

/// A table cell being typed into.
struct CellEdit {
    /// The bytes the cell stands for in the document.
    span: Range<usize>,
    /// Those bytes when the field opened; a commit writes nothing if they
    /// are no longer there.
    was: String,
    field: Field,
}

/// Results of the last decoration pass, shown in the status bar.
#[derive(Debug, Clone, Copy, Default)]
pub struct ParseStats {
    pub parse_ms: f64,
    pub spans: usize,
    pub bytes: usize,
    pub words: usize,
    pub chars: usize,
}

/// Running totals for the stress run: count, sum and maximum of a duration.
#[derive(Debug, Clone, Copy, Default)]
pub struct Sample {
    pub count: u64,
    pub total_ms: f64,
    pub max_ms: f64,
}

impl Sample {
    fn add(&mut self, ms: f64) {
        self.count += 1;
        self.total_ms += ms;
        if ms > self.max_ms {
            self.max_ms = ms;
        }
    }

    pub fn mean_ms(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.total_ms / self.count as f64
        }
    }
}

/// The find and replace panel: its two fields, the matches of the query
/// over the buffer, and which one is selected.
struct FindState {
    open: bool,
    /// The replace row is shown (Ctrl+H, or the toggle).
    replace_shown: bool,
    query: Entity<TextInput>,
    replacement: Entity<TextInput>,
    /// Match case and regex, both off by default.
    opts: find::Options,
    matches: Vec<Range<usize>>,
    current: Option<usize>,
}

/// Colors and fonts copied from the theme at render time, so shaping
/// outside a frame (clicks, cursor movement) matches what was painted.
#[derive(Clone)]
struct Palette {
    text: Hsla,
    muted: Hsla,
    accent: Hsla,
    link: Hsla,
    tag: Hsla,
    code_bg: Hsla,
    highlight_bg: Hsla,
    selection: Hsla,
    border: Hsla,
    /// Every match of the find query, and the selected one.
    find_match: Hsla,
    find_current: Hsla,
    /// A task's box: `--checkbox-border-color` while open, `--checkbox-color`
    /// filled when checked, the tick in `--checkbox-marker-color`, as the
    /// reading view draws it.
    checkbox: Hsla,
    checkbox_border: Hsla,
    checkbox_marker: Hsla,
    mono: SharedString,
}

impl Palette {
    fn from_theme(t: &Theme) -> Palette {
        let mut selection = t.accent;
        selection.a = if t.dark { 0.35 } else { 0.25 };
        Palette {
            text: t.text,
            muted: t.text_muted,
            accent: t.accent,
            link: t.link,
            tag: t.tag,
            code_bg: t.code_bg,
            highlight_bg: t.highlight_bg,
            selection,
            border: t.border,
            find_match: {
                let mut c = t.highlight_bg;
                c.a = 0.3;
                c
            },
            find_current: {
                let mut c = t.highlight_bg;
                c.a = 0.75;
                c
            },
            checkbox: t.checkbox,
            checkbox_border: t.checkbox_border,
            checkbox_marker: t.checkbox_marker,
            mono: t.mono_font.clone(),
        }
    }
}

impl Default for Palette {
    fn default() -> Palette {
        Palette::from_theme(&Theme::dark())
    }
}

/// A line painted in the last frame: where it sat and how many rows it wrapped to.
#[derive(Debug, Clone, Copy)]
struct VisibleLine {
    line: usize,
    /// Top of the line relative to the element's origin.
    top: Pixels,
    rows: usize,
    /// Height of each row: the body line height, more for a heading.
    row_height: Pixels,
}

impl VisibleLine {
    fn height(&self) -> Pixels {
        self.row_height * self.rows as f32
    }
}

/// One line shaped for the screen, with the mapping between its source
/// bytes and the display text that was shaped (they differ in Live Preview).
pub(crate) struct Shaped {
    display: DisplayLine,
    line: WrappedLine,
    row_height: Pixels,
}

impl Shaped {
    fn rows(&self) -> usize {
        self.line.wrap_boundaries().len() + 1
    }

    fn height(&self) -> Pixels {
        self.row_height * self.rows() as f32
    }

    /// The display byte each wrapped row after the first begins at.
    fn row_starts(&self) -> impl Iterator<Item = usize> + '_ {
        let layout = &self.line.unwrapped_layout;
        self.line
            .wrap_boundaries()
            .iter()
            .map(move |b| layout.runs[b.run_ix].glyphs[b.glyph_ix].index)
    }

    /// The wrapped row and x of source byte `col`.
    fn row_and_x(&self, col: usize) -> (usize, Pixels) {
        if self.row_height <= px(0.) {
            return (0, px(0.));
        }
        let d = self.display.to_display(col);
        if self.row_starts().any(|start| start == d) {
            // * gpui reports a byte that falls on a wrap point at the right
            //   edge of the row before, which strands the caret a row above
            //   the text it is in and leaves Home with nowhere for End to go.
            return (wrap::row_of(self.row_starts(), d), px(0.));
        }
        match self.line.position_for_index(d, self.row_height) {
            Some(p) => ((p.y / self.row_height).round() as usize, p.x),
            None => (0, px(0.)),
        }
    }

    /// The display byte nearest `x` on `row`.
    fn display_index_at(&self, x: Pixels, row: usize) -> usize {
        let row = row.min(self.rows() - 1);
        self.line
            .closest_index_for_position(
                point(x.max(px(0.)), self.row_height * row as f32),
                self.row_height,
            )
            .unwrap_or_else(|i| i)
    }

    /// The display byte of the glyph `x` sits inside on `row`, or `None`
    /// when the point is left of the text or past the end of the row.
    ///
    /// A click on a rendered construct asks this rather than
    /// `display_index_at`, which clamps an out-of-range point onto the
    /// nearest character and so would hand a click in the left margin, or
    /// in the blank space after the last glyph, to whatever construct
    /// happens to sit at that end of the line.
    fn glyph_index_at(&self, x: Pixels, row: usize) -> Option<usize> {
        if row >= self.rows() {
            return None;
        }
        self.line
            .index_for_position(point(x, self.row_height * row as f32), self.row_height)
            .ok()
    }

    /// The source byte nearest `x` on `row`.
    fn index_at(&self, x: Pixels, row: usize) -> usize {
        self.to_source(self.display_index_at(x, row))
    }

    /// The source byte display byte `d` stands for.
    fn to_source(&self, d: usize) -> usize {
        source_index(&self.display, d)
    }
}

/// The source byte display byte `d` stands for.
///
/// `DisplayLine::to_source` answers with the piece that spans `d`, and
/// markup hidden at the end of a line is a substitution that displays
/// nothing, so it spans nothing and the end of the display falls back
/// into the copied piece before it — inside the construct that markup
/// closes. The end of the display is the end of the source line instead,
/// which is where a click past the last glyph of `see [[Welcome]]` should
/// put the caret: after the `]]`, not between `Welcome` and it.
fn source_index(display: &DisplayLine, d: usize) -> usize {
    if d < display.text.len() {
        return display.to_source(d);
    }
    display
        .pieces
        .iter()
        .map(|p| match p {
            Piece::Copy { source, len, .. } => source + len,
            Piece::Sub { source, .. } => source.end,
        })
        .max()
        .unwrap_or(0)
}

/// Where a mouse point landed, resolved against the last frame's layout.
pub(crate) struct Hit {
    line: usize,
    shaped: Shaped,
    /// The display byte nearest the point: where a click puts the caret,
    /// clamped onto the line however far outside the point was.
    display: usize,
    /// The display byte of the glyph under the point, when the point was
    /// on one at all. `None` for the margins and for the blank space past
    /// the end of a row, which no construct is drawn in.
    glyph: Option<usize>,
}

/// A Live Preview widget as painted: the lines it stands for and where.
#[derive(Debug, Clone)]
struct WidgetBox {
    lines: Range<usize>,
    bounds: Bounds<Pixels>,
    /// An embed keeps its clicks (links inside it work); a table or math
    /// block opens for editing when clicked.
    embed: bool,
}

/// Geometry recorded at paint time so input handling can map points to text.
#[derive(Clone, Default)]
struct Layout {
    bounds: Bounds<Pixels>,
    /// Widgets painted in the last frame.
    widgets: Vec<WidgetBox>,
    line_height: Pixels,
    font_size: Pixels,
    text_style: Option<TextStyle>,
    wrap_width: Option<Pixels>,
    /// Left edge of the text.
    content_x: Pixels,
    visible: Vec<VisibleLine>,
    cursor_line: Option<Arc<Shaped>>,
    /// Top of the cursor's line in window coordinates.
    cursor_top: Pixels,
    /// The scrollbar's track and thumb, when the note overflows the view.
    scrollbar: Option<Scrollbar>,
    /// Time spent shaping the visible lines in the last frame.
    prepaint_ms: f64,
}

/// Vertical scrollbar geometry in window coordinates. Positions are
/// line-based: the thumb sits at the first visible line's share of the
/// lines the view can start at, and is as tall as the share of lines the
/// last screenful holds, so no full layout pass is ever needed. Both are
/// fixed for a given note and view, so the thumb keeps its size as it
/// moves; the lines on screen at any one scroll position are not (wrapped
/// lines and widgets come and go) and must not drive it.
#[derive(Debug, Clone, Copy)]
struct Scrollbar {
    track: Bounds<Pixels>,
    thumb: Bounds<Pixels>,
    /// Lines the thumb can be dragged across: the last line the view can
    /// start at before the note's end reaches its bottom.
    scrollable_lines: usize,
}

impl Scrollbar {
    /// Lays the thumb out for a view starting `first` lines (fractional,
    /// for the part of a line scrolled off the top) into `count` lines,
    /// where `last_top` is the last line the view can start at; `None`
    /// when everything fits.
    fn layout(
        track: Bounds<Pixels>,
        count: usize,
        first: f32,
        last_top: usize,
        fits: bool,
    ) -> Option<Scrollbar> {
        if fits || last_top == 0 || count <= last_top || track.size.height <= px(0.) {
            return None;
        }
        let track_h = track.size.height;
        let end_lines = count - last_top;
        let thumb_h = (track_h * (end_lines as f32 / count as f32))
            .max(px(SCROLLBAR_MIN_THUMB))
            .min(track_h);
        let scrollable_lines = last_top;
        let frac = (first / scrollable_lines as f32).clamp(0., 1.);
        let top = track.origin.y + (track_h - thumb_h) * frac;
        let inset = px(3.);
        Some(Scrollbar {
            track,
            thumb: Bounds::new(
                point(track.origin.x + inset, top),
                size(track.size.width - inset * 2., thumb_h),
            ),
            scrollable_lines,
        })
    }

    /// The first visible line for a thumb whose top is at `y`.
    fn line_for_thumb_top(&self, y: Pixels) -> usize {
        let room = self.track.size.height - self.thumb.size.height;
        if room <= px(0.) {
            return 0;
        }
        let frac = ((y - self.track.origin.y) / room).clamp(0., 1.);
        (frac * self.scrollable_lines as f32).round() as usize
    }
}

pub struct Editor {
    focus_handle: FocusHandle,
    /// The vault-relative path of the file in the buffer, empty when no
    /// file is behind it. The shell reads it that way: an empty name is
    /// what tells it to paint its empty state instead of this view, and to
    /// save nothing.
    pub name: SharedString,
    /// The text, its line index, the selections and the undo history; the
    /// bytes this view writes back are all decided there.
    buf: TextBuffer,
    scroll_line: usize,
    /// Pixels of `scroll_line` scrolled above the top of the view.
    scroll_px: Pixels,
    /// Reveal the cursor in the next frame (set when no window was at hand).
    pending_reveal: bool,
    /// The pending reveal puts the cursor's row at the vertical centre of
    /// the view rather than scrolling the least that shows it: what an
    /// Outline row's click does in Obsidian.
    pending_center: bool,
    dragging: bool,
    /// Dragging the scrollbar thumb: the grab point's offset from its top.
    thumb_drag: Option<Pixels>,
    find: FindState,
    _subscriptions: Vec<Subscription>,
    highlights: Arc<Vec<Vec<HighlightSpan>>>,
    /// Live Preview's block widgets with the lines each covers, from the
    /// same parse as the highlights.
    widgets: Arc<Vec<(Range<usize>, Widget)>>,
    /// Live Preview's code-block flairs by opening line, from that same
    /// parse. Only Live Preview paints them, which is where Obsidian has
    /// them: source mode showed none at all.
    flairs: Arc<Vec<Flair>>,
    /// Height of each widget as last painted, by first line; an unpainted
    /// one is taken as one row per source line.
    widget_heights: HashMap<usize, Pixels>,
    /// The no-break spaces inline math is shaped as, measured in the text
    /// font at the font size they were measured for; `None` before the
    /// first frame, when nothing has been measured and inline math shows
    /// its source with the dollars hidden.
    spaces: Option<(Pixels, SpaceWidths)>,
    /// Resolves embeds for the widgets; `None` before a vault is open.
    embed_source: Option<Arc<dyn EmbedSource + Send + Sync>>,
    build_options: BuildOptions,
    /// The vault's `types.json`, for the type each property row shows.
    property_types: PropertyTypes,
    /// `propertiesInDocument`: the table, nothing, or the raw YAML.
    properties_in_document: PropertiesInDocument,
    /// The properties field being typed into, if any. While it is open the
    /// editor's keys go to the field instead of the document, and the
    /// document changes once on commit rather than per keystroke.
    property_field: Option<Field>,
    /// The pill a Backspace in the open list field selected, by item index.
    /// Obsidian's Backspace takes two presses — the first moves focus from
    /// the trailing input to the last pill, the second removes it — so the
    /// selection between them lives here. It belongs to `property_field`
    /// and goes with it, and anything but another Backspace clears it.
    selected_pill: Option<usize>,
    /// Whether the properties block is collapsed.
    properties_folded: bool,
    /// Callouts whose fold state the reader flipped, by title id — the same
    /// set the reading view keeps, and with the same caveat: the ids are the
    /// model builder's counters, so they only mean anything for the note
    /// that produced them.
    callout_folds: HashSet<usize>,
    /// The lines that can fold, from the same parse as the highlights.
    foldables: Arc<Vec<Foldable>>,
    /// The heading and list folds of this note, by key; resolved against
    /// `foldables` after every parse.
    folds: fold::Folds,
    /// The hidden line runs the folds come to, `(first, last)` inclusive.
    hidden: Vec<(usize, usize)>,
    /// A record restored from the store before the first parse delivered
    /// the foldables it is resolved against.
    pending_folds: Option<Vec<usize>>,
    /// The line under the pointer, for the fold indicator it reveals.
    hover_line: Option<usize>,
    /// The property whose type menu is open.
    property_menu: Option<String>,
    /// The table cell being typed into. Like a properties field, the keys
    /// go to it while it is open and the document changes once, on commit.
    table_field: Option<CellEdit>,
    /// The open autocompletion popup, with the rows it is offering.
    suggest: Option<SuggestState>,
    /// Hover page preview card on Ctrl+hover.
    hover_preview: Option<(Point<Pixels>, crate::page_preview::PagePreviewData)>,
    /// The vault the suggester reads; `None` until one is open.
    suggest_vault: Option<Arc<VaultState>>,
    /// `newLinkFormat` and `useMarkdownLinks`, for the links it writes.
    link_options: suggest::Options,
    generation: u64,
    pub stats: ParseStats,
    layout: Layout,
    palette: Palette,
    pending_parse: Option<Task<()>>,
    /// An edit arrived while a parse was running; parse again when it lands.
    parse_queued: bool,
    /// Byte ranges that changed since the last parse completed. The
    /// foundation for incremental parsing: once the parser can re-parse a
    /// substring and splice it into the existing tree, these ranges tell it
    /// what to touch instead of the whole document.
    dirty_ranges: Vec<std::ops::Range<usize>>,
    readable_line_length: bool,
    /// Whether line numbers are painted in the gutter (`showLineNumber`).
    show_line_number: bool,
    /// Whether vertical indent guides are painted under indented lines
    /// (`showIndentGuide`).
    show_indent_guide: bool,
    /// Whether Vim key bindings are active (`vimMode`). When on, the editor
    /// starts in normal mode and keystrokes are Vim commands until `i` or
    /// `a` enters insert mode; Escape returns to normal mode.
    vim_mode: bool,
    /// Whether the editor is currently in Vim insert mode. Only meaningful
    /// when `vim_mode` is on.
    vim_insert_mode: bool,
    /// Whether spellchecking is on (`spellcheck`). When on, words not found
    /// in the dictionary are highlighted as misspelled.
    spellcheck: bool,
    /// Whether text direction is right-to-left (`rightToLeft`). When on,
    /// lines are aligned to the right and shaped RTL.
    right_to_left: bool,
    /// The spell checker: dictionary and misspelled-word positions.
    spellchecker: SpellChecker,
    /// Live Preview: markers hidden and constructs rendered in place,
    /// except where the cursor is.
    live_preview: bool,
    /// Enter continues list and quote markers (`smartIndentList`).
    smart_indent_list: bool,
    /// Tab inserts a tab (`useTab`) or `tab_size` spaces.
    use_tab: bool,
    tab_size: u8,
    /// Shaping time per painted frame since the note was opened.
    pub frame_samples: Sample,
    /// Parse time per completed decoration pass since the note was opened.
    pub parse_samples: Sample,
    /// Edit-to-decoration latency, including waiting for the background thread.
    pub parse_latency: Sample,
    /// The right-click context menu over the editor, if one is open.
    context_menu: Option<ContextMenu>,
}

/// A right-click context menu over the editor.
struct ContextMenu {
    /// Where the menu should appear, in viewport pixels.
    position: Point<Pixels>,
    /// What was right-clicked: the byte offset, or None for empty space.
    target: Option<usize>,
}

/// One context menu row: label, icon, and the command it emits.
#[derive(Clone)]
struct MenuItem {
    label: &'static str,
    icon: &'static str,
    event: EditorEvent,
}

impl EventEmitter<EditorEvent> for Editor {}

impl Editor {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Editor {
        let query = cx.new(|cx| TextInput::new("Find", cx));
        let replacement = cx.new(|cx| TextInput::new("Replace", cx));
        let subscriptions =
            vec![
                cx.subscribe_in(&query, window, |this, _, event: &InputEvent, window, cx| {
                    match event {
                        InputEvent::Changed => this.query_changed(cx),
                        InputEvent::Submit => this.find_step(true, cx),
                        InputEvent::Cancel => this.close_find(&CloseFind, window, cx),
                        InputEvent::Up | InputEvent::Down => {}
                    }
                }),
                cx.subscribe_in(
                    &replacement,
                    window,
                    |this, _, event: &InputEvent, window, cx| match event {
                        InputEvent::Submit => this.replace_next(cx),
                        InputEvent::Cancel => this.close_find(&CloseFind, window, cx),
                        InputEvent::Changed | InputEvent::Up | InputEvent::Down => {}
                    },
                ),
            ];
        Editor {
            focus_handle: cx.focus_handle(),
            find: FindState {
                open: false,
                replace_shown: false,
                query,
                replacement,
                opts: find::Options::default(),
                matches: Vec::new(),
                current: None,
            },
            _subscriptions: subscriptions,
            // * A view that has not been given a file yet belongs to none,
            //   and a placeholder name here would read as one: the shell
            //   would mount this editor on launch over an empty vault and
            //   take a whole note's worth of typing that nothing saves.
            name: SharedString::default(),
            buf: TextBuffer::new(),
            scroll_line: 0,
            scroll_px: px(0.),
            pending_reveal: false,
            pending_center: false,
            dragging: false,
            thumb_drag: None,
            highlights: Arc::new(vec![Vec::new()]),
            widgets: Arc::new(Vec::new()),
            flairs: Arc::new(Vec::new()),
            widget_heights: HashMap::new(),
            spaces: None,
            embed_source: None,
            build_options: BuildOptions::default(),
            property_types: PropertyTypes::default(),
            properties_in_document: PropertiesInDocument::default(),
            property_field: None,
            selected_pill: None,
            properties_folded: false,
            callout_folds: HashSet::new(),
            foldables: Arc::new(Vec::new()),
            folds: fold::Folds::default(),
            hidden: Vec::new(),
            pending_folds: None,
            hover_line: None,
            property_menu: None,
            table_field: None,
            suggest: None,
            hover_preview: None,
            suggest_vault: None,
            link_options: suggest::Options::default(),
            generation: 0,
            stats: ParseStats::default(),
            layout: Layout::default(),
            palette: Palette::default(),
            pending_parse: None,
            parse_queued: false,
            dirty_ranges: Vec::new(),
            readable_line_length: true,
            show_line_number: false,
            show_indent_guide: true,
            vim_mode: false,
            vim_insert_mode: false,
            spellcheck: false,
            right_to_left: false,
            spellchecker: SpellChecker::new(),
            live_preview: false,
            smart_indent_list: true,
            use_tab: true,
            tab_size: 4,
            frame_samples: Sample::default(),
            parse_samples: Sample::default(),
            parse_latency: Sample::default(),
            context_menu: None,
        }
    }

    /// Replaces the buffer with a file's raw text, remembering its BOM and
    /// line endings; the cursor goes to the start.
    pub fn set_text(&mut self, name: impl Into<SharedString>, raw: String, cx: &mut Context<Self>) {
        self.name = name.into();
        self.buf.load(&raw);
        self.suggest = None;
        self.hover_preview = None;
        self.table_field = None;
        // * Fold ids are per-note counters, so another note's would collapse
        //   whichever callout happened to land on the same number.
        self.callout_folds.clear();
        self.foldables = Arc::new(Vec::new());
        self.folds = fold::Folds::default();
        self.hidden.clear();
        self.pending_folds = None;
        self.hover_line = None;
        self.properties_folded = false;
        self.buf.head = 0;
        self.buf.anchor = 0;
        self.widgets = Arc::new(Vec::new());
        self.flairs = Arc::new(Vec::new());
        self.widget_heights.clear();
        self.scroll_line = 0;
        self.scroll_px = px(0.);
        self.frame_samples = Sample::default();
        self.parse_samples = Sample::default();
        self.parse_latency = Sample::default();
        // * The matches point into the note that was open; drop them rather
        //   than let the panel count or step through another note's offsets.
        self.find.matches.clear();
        self.find.current = None;
        self.refresh_matches(cx);
        self.request_parse(cx);
        cx.notify();
    }

    /// The buffer's file moved: keep the text, remember the new path.
    pub fn set_name(&mut self, name: impl Into<SharedString>) {
        self.name = name.into();
    }

    /// Replaces the buffer after the file changed on disk, keeping the
    /// cursor and scroll position where they were (clamped to the new text).
    pub fn reload(&mut self, raw: String, cx: &mut Context<Self>) {
        let (head, anchor) = (self.buf.head, self.buf.anchor);
        self.suggest = None;
        self.table_field = None;
        self.buf.load(&raw);
        self.buf.head = self.buf.clamp_offset(head);
        self.buf.anchor = self.buf.clamp_offset(anchor);
        self.scroll_line = self.scroll_line.min(self.line_count() - 1);
        self.request_parse(cx);
        cx.notify();
    }

    /// The bytes to write back: the buffer with the file's original line
    /// endings and byte order mark.
    pub fn file_contents(&self) -> String {
        self.buf.file_contents()
    }

    pub fn is_dirty(&self) -> bool {
        self.buf.is_dirty()
    }

    /// The buffer's text, `\n`-ended and without the file's byte order
    /// mark. The reading view is rebuilt from this when the workspace
    /// edits the note while the reader, not the editor, is on screen.
    pub fn text(&self) -> &str {
        self.buf.text()
    }

    /// Whether `raw` (a file's bytes) is this buffer, ignoring the BOM and
    /// line endings the buffer normalizes away.
    pub fn matches_file(&self, raw: &str) -> bool {
        self.buf.matches_file(raw)
    }

    /// Records that `contents` is what the file now holds.
    pub fn mark_saved(&mut self, contents: &str) {
        self.buf.mark_saved();
        self.buf.mark_on_disk(contents);
    }

    /// Whether `raw` is the bytes this note was loaded from or last saved
    /// as. A save that finds anything else has a change it has not seen,
    /// and the vault is the source of truth.
    pub fn disk_unchanged(&self, raw: &str) -> bool {
        self.buf.disk_unchanged(raw)
    }

    pub fn set_readable_line_length(&mut self, on: bool) {
        self.readable_line_length = on;
    }

    pub fn set_show_line_number(&mut self, on: bool) {
        self.show_line_number = on;
    }

    pub fn set_show_indent_guide(&mut self, on: bool) {
        self.show_indent_guide = on;
    }

    pub fn set_vim_mode(&mut self, on: bool) {
        self.vim_mode = on;
        if !on {
            self.vim_insert_mode = false;
        }
    }

    pub fn set_spellcheck(&mut self, on: bool) {
        self.spellcheck = on;
    }

    /// Toggles spellchecking on or off (`editor:toggle-spellcheck`).
    pub fn toggle_spellcheck(&mut self) {
        self.spellcheck = !self.spellcheck;
    }

    pub fn toggle_show_line_number(&mut self) {
        self.show_line_number = !self.show_line_number;
    }

    pub fn toggle_readable_line_length(&mut self) {
        self.readable_line_length = !self.readable_line_length;
    }

    pub fn set_right_to_left(&mut self, on: bool) {
        self.right_to_left = on;
    }

    /// Switches between Live Preview and Source mode. Widgets are only
    /// built for Live Preview, so switching to it parses again.
    pub fn set_live_preview(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.live_preview != on {
            self.live_preview = on;
            self.pending_reveal = true;
            if on {
                self.request_parse(cx);
            }
            cx.notify();
        }
    }

    /// Where Live Preview's embeds come from, and the vault's rendering
    /// settings; set once a vault is open.
    pub fn set_vault(
        &mut self,
        source: Option<Arc<dyn EmbedSource + Send + Sync>>,
        opts: BuildOptions,
        cx: &mut Context<Self>,
    ) {
        self.embed_source = source;
        self.build_options = opts;
        self.request_parse(cx);
    }

    /// New render options (`strictLineBreaks` changed in Settings); the
    /// widgets are rebuilt with them.
    pub fn set_build_options(&mut self, opts: BuildOptions, cx: &mut Context<Self>) {
        if self.build_options != opts {
            self.build_options = opts;
            self.request_parse(cx);
        }
    }

    /// The vault's `types.json`, for the properties widget's type glyphs.
    pub fn set_property_types(&mut self, types: PropertyTypes) {
        self.property_types = types;
    }

    /// `propertiesInDocument` from `app.json`.
    pub fn set_properties_in_document(&mut self, mode: PropertiesInDocument) {
        self.properties_in_document = mode;
    }

    /// The vault the autocompletion popup offers rows from. The state is
    /// read through its locks on every keystroke, so a file added or
    /// renamed while the note is open is offered without setting it again.
    pub fn set_suggest_vault(&mut self, state: Arc<VaultState>) {
        self.suggest_vault = Some(state);
    }

    /// `newLinkFormat` and `useMarkdownLinks`: the shape of the links an
    /// accepted suggestion writes.
    pub fn set_link_options(&mut self, opts: suggest::Options) {
        self.link_options = opts;
    }

    /// Editor settings from `app.json`: smart lists, and tabs or spaces.
    pub fn set_indent_options(&mut self, smart_indent_list: bool, use_tab: bool, tab_size: u8) {
        self.smart_indent_list = smart_indent_list;
        self.use_tab = use_tab;
        self.tab_size = tab_size;
    }

    /// `autoPairBrackets` and `autoPairMarkdown`, read from the vault's
    /// `app.json` as each character is typed. The Settings modal writes
    /// the config through the same lock, so a toggle takes effect on the
    /// next keystroke without the note being reopened.
    fn auto_pair(&self) -> autopair::Options {
        let Some(state) = &self.suggest_vault else {
            return autopair::Options::default();
        };
        let app = state.app();
        autopair::Options {
            brackets: app.auto_pair_brackets.unwrap_or(true),
            markdown: app.auto_pair_markdown.unwrap_or(true),
        }
    }

    /// Whether the cursor is in a table: the table commands are offered
    /// only then, the way Obsidian's check callback hides them.
    pub fn in_table(&self) -> bool {
        self.buf.in_table()
    }

    /// Runs one of Obsidian's table commands at the cursor, rewriting the
    /// whole table as Obsidian does. `false` when it does not apply.
    pub fn table_command(
        &mut self,
        cmd: TableCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.buf.table_command(cmd) {
            return false;
        }
        self.after_edit(Some(window), cx, None);
        true
    }

    /// `editor:insert-table`: an empty two-by-two table after the cursor's
    /// line, with the cursor in its first header cell.
    pub fn insert_table(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.buf.insert_table() {
            self.after_edit(Some(window), cx, None);
        }
    }

    /// One of Obsidian's formatting toggles: wraps each selection in
    /// `marker`, or takes the marker off when it is already there.
    pub fn toggle_wrap(
        &mut self,
        marker: &str,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        if self.buf.toggle_wrap(marker) {
            self.after_edit(window, cx, None);
        }
    }

    /// `editor:toggle-checklist-status` (Mod+L): a checkbox on every
    /// selected line that has none, or the one it has flipped, writing
    /// the same bytes a click on the rendered checkbox writes.
    pub fn toggle_checklist(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        if self.buf.toggle_checklist() {
            self.after_edit(window, cx, None);
        }
    }

    /// Cut the selection to the clipboard (called from the context menu).
    pub fn do_cut(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        if self.field_kind().is_some() {
            return;
        }
        let ranges = self.buf.copy_ranges();
        if ranges.iter().any(|r| !r.is_empty()) {
            cx.write_to_clipboard(ClipboardItem::new_string(self.buf.clipboard_text(&ranges)));
            let edits: Vec<(Range<usize>, String)> =
                ranges.into_iter().map(|r| (r, String::new())).collect();
            self.edit_all(edits, EditKind::Other, window, cx);
        }
    }

    /// Copy the selection to the clipboard (called from the context menu).
    pub fn do_copy(&mut self, _window: Option<&mut Window>, cx: &mut Context<Self>) {
        if self.field_kind().is_some() {
            return;
        }
        let ranges = self.buf.copy_ranges();
        if ranges.iter().any(|r| !r.is_empty()) {
            cx.write_to_clipboard(ClipboardItem::new_string(self.buf.clipboard_text(&ranges)));
        }
    }

    /// Paste from the clipboard (called from the context menu).
    pub fn do_paste(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        if let Some(field) = self.active_field() {
            if let Some(text) = item.text() {
                field.insert(&text);
                cx.notify();
            }
            return;
        }
        if let Some(text) = item.text() {
            if self.buf.paste(&text) {
                self.after_edit(window, cx, None);
            }
        }
    }

    /// Select all text (called from the context menu).
    pub fn do_select_all(&mut self, cx: &mut Context<Self>) {
        let len = self.buf.len();
        if len > 0 {
            self.buf.head = len;
            self.buf.anchor = 0;
            cx.notify();
        }
    }

    /// Toggle a bullet list on the current line(s).
    pub fn toggle_list(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        if self.buf.toggle_line_prefix(LinePrefix::Bullet) {
            self.after_edit(window, cx, None);
        }
    }

    /// Toggle a blockquote on the current line(s).
    pub fn toggle_blockquote(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        if self.buf.toggle_line_prefix(LinePrefix::Quote) {
            self.after_edit(window, cx, None);
        }
    }

    /// `editor:cycle-list-checklist`: bullet, `[ ]`, `[x]`, bullet.
    pub fn cycle_checklist(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.buf.cycle_checklist() {
            self.after_edit(Some(window), cx, None);
        }
    }

    /// `editor:toggle-highlight-<colour>`: a highlight with its colour
    /// emoji after the opening `==`.
    pub fn toggle_highlight(
        &mut self,
        color: Option<silica_markdown::HighlightColor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.buf.toggle_wrap_colored("==", color) {
            self.after_edit(Some(window), cx, None);
        }
    }

    /// `editor:clear-formatting`: the markers inside the selection go.
    pub fn clear_formatting(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.buf.clear_formatting() {
            self.after_edit(Some(window), cx, None);
        }
    }

    /// One of the six insert-block commands at the caret.
    pub fn insert_block(&mut self, kind: BlockInsert, window: &mut Window, cx: &mut Context<Self>) {
        if self.buf.insert_block(kind) {
            self.after_edit(Some(window), cx, None);
        }
    }

    /// `editor:set-heading-<n>`: rewrites the selected lines' `#`s.
    pub fn set_heading(&mut self, level: u8, window: Option<&mut Window>, cx: &mut Context<Self>) {
        if self.buf.set_heading(level) {
            self.after_edit(window, cx, None);
        }
    }

    /// `editor:toggle-bullet-list`, `-numbered-list` and `-blockquote`.
    pub fn toggle_line_prefix(
        &mut self,
        kind: LinePrefix,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.buf.toggle_line_prefix(kind) {
            self.after_edit(Some(window), cx, None);
        }
    }

    /// `editor:delete-paragraph`.
    pub fn delete_paragraph(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.buf.delete_paragraph() {
            self.after_edit(Some(window), cx, None);
        }
    }

    /// `editor:swap-line-up` and `-down`.
    pub fn swap_lines(&mut self, up: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.buf.swap_lines(up) {
            self.after_edit(Some(window), cx, None);
        }
    }

    /// `editor:insert-link`, `-wikilink` and `-embed`: the link shape
    /// around the selection, with the caret where the target is typed —
    /// which is where `after_edit` opens the autocompletion popup.
    pub fn insert_link(&mut self, kind: LinkSyntax, window: &mut Window, cx: &mut Context<Self>) {
        if self.buf.insert_link(kind) {
            self.after_edit(Some(window), cx, None);
        }
    }

    /// `editor:indent-list` and `-unindent-list`: the same step Tab and
    /// Shift+Tab take.
    pub fn indent_list(&mut self, indent: bool, window: &mut Window, cx: &mut Context<Self>) {
        let moved = if indent {
            self.buf
                .indent(self.use_tab, self.tab_size, self.smart_indent_list)
        } else {
            self.buf
                .outdent(self.use_tab, self.tab_size, self.smart_indent_list)
        };
        if moved {
            self.after_edit(Some(window), cx, None);
        }
    }

    /// `editor:follow-link`: opens the link the cursor is in, the way
    /// Ctrl+click opens the one under the pointer. False when the cursor
    /// is not in a link.
    pub fn follow_link_at_cursor(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(target) = self.link_target_at_cursor() else {
            return false;
        };
        cx.emit(EditorEvent::OpenLink(target));
        true
    }

    /// The link under the cursor, for a caller that opens it somewhere
    /// other than this editor's own tab.
    pub fn link_target_at_cursor(&self) -> Option<LinkTarget> {
        let cursor = self.cursor();
        // * Not `rendered_only`: a cursor inside a link is exactly what
        //   makes Live Preview reveal its source.
        self.link_at(cursor.line, cursor.col, false)
    }

    /// Types `text` at the cursor the way a keystroke does, so the
    /// autocompletion popup opens for it. Used by `--type` to put the popup
    /// on screen for a screenshot, and by the control socket, which drives
    /// the app in place of a keyboard: one character at a time through the
    /// same insertion, so the two cannot disagree about auto-pairing.
    pub fn type_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(field) = self.active_field() {
            field.insert(text);
            cx.notify();
            return;
        }
        let mut utf8 = [0u8; 4];
        for ch in text.chars() {
            self.type_char(ch.encode_utf8(&mut utf8), EditKind::Typing, window, cx);
        }
    }

    /// Writes `text` at every cursor as one undo step, which is how a
    /// template and the two date commands reach the document.
    pub fn insert_at_cursors(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.buf.insert_at_cursors(text) {
            self.after_edit(Some(window), cx, None);
        }
    }

    /// Presses keys by name, for `--press`: the popup's accept keys, and
    /// the keys that move between table cells, so a screenshot can show
    /// what a keystroke leaves behind.
    pub fn press_keys(&mut self, keys: &str, window: &mut Window, cx: &mut Context<Self>) {
        for key in keys.split(',') {
            match key.trim() {
                "down" => {
                    self.move_down(&MoveDown, window, cx);
                }
                "up" => {
                    self.move_up(&MoveUp, window, cx);
                }
                "tab" => self.tab(&Tab, window, cx),
                "shift-tab" => self.outdent(&Outdent, window, cx),
                "shift-enter" => self.newline_plain(&NewlinePlain, window, cx),
                "escape" => self.close_find(&CloseFind, window, cx),
                _ => self.newline(&Newline, window, cx),
            }
        }
    }

    /// Opens cell `ix` of the first table in the note for editing, for
    /// `--cell`; clicking one is what does it in the app.
    pub fn edit_cell(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(span) = table::tables(self.buf.text())
            .first()
            .and_then(|t| table::table_at(self.buf.text(), t.start))
            .and_then(|at| at.cells.iter().flatten().nth(ix).cloned())
        else {
            return;
        };
        self.table_action(TableAction::Edit { span }, window, cx);
    }

    /// One step of the self-driving stress run: types, breaks lines, pages
    /// and jumps through the document the way a user would, only faster.
    pub fn stress_step(&mut self, step: usize, window: &mut Window, cx: &mut Context<Self>) {
        match step % 16 {
            0..=5 => self.replace_text_in_range(None, "word ", window, cx),
            6 => self.newline(&Newline, window, cx),
            7 => self.page_down(&PageDown, window, cx),
            8 => self.move_down(&MoveDown, window, cx),
            9 => self.backspace(&Backspace, window, cx),
            10 => {
                let line = (step * 7919) % self.line_count();
                let at = self.buf.line_starts()[line];
                self.move_to(at, false, window, cx);
            }
            11 => self.move_up(&MoveUp, window, cx),
            12 => self.select_line_end(&SelectLineEnd, window, cx),
            13 => self.select_down(&SelectDown, window, cx),
            14 => self.delete(&Delete, window, cx),
            _ => self.undo(&Undo, window, cx),
        }
    }

    /// Summary of the stress run for the terminal.
    pub fn report(&self) -> String {
        format!(
            "frames {}: shape mean {:.2} ms, max {:.2} ms\nparses {}: parse mean {:.1} ms, max {:.1} ms\nedit-to-decoration latency mean {:.1} ms, max {:.1} ms\n{} lines, {} KB, {} spans",
            self.frame_samples.count,
            self.frame_samples.mean_ms(),
            self.frame_samples.max_ms,
            self.parse_samples.count,
            self.parse_samples.mean_ms(),
            self.parse_samples.max_ms,
            self.parse_latency.mean_ms(),
            self.parse_latency.max_ms,
            self.line_count(),
            self.stats.bytes / 1024,
            self.stats.spans
        )
    }

    pub fn cursor(&self) -> Cursor {
        self.buf.cursor()
    }

    /// The primary selection's byte range, empty at the cursor when
    /// nothing is selected.
    pub fn selection(&self) -> Range<usize> {
        self.buf.selection()
    }

    /// How many cursors there are (one unless Find all or Alt+click added more).
    pub fn cursor_count(&self) -> usize {
        self.buf.cursor_count()
    }

    pub fn line_count(&self) -> usize {
        self.buf.line_count()
    }

    /// Milliseconds the last frame spent shaping visible lines.
    pub fn prepaint_ms(&self) -> f64 {
        self.layout.prepaint_ms
    }

    // ----- the control socket ---------------------------------------------------

    /// The `editor` part of the control socket's state dump: what the
    /// buffer holds, where the cursor is, and what the panels are showing.
    ///
    /// Everything here is read back out of the buffer and the last frame's
    /// layout rather than from state kept alongside them, so a dump can
    /// never disagree with what the keystrokes before it left behind.
    pub fn describe(&self, text: bool, cx: &App) -> serde_json::Value {
        let cursor = self.buf.cursor();
        let sel = self.buf.selection();
        let (first, last) = self.visible_line_range();
        let mut out = serde_json::json!({
            "name": self.name.to_string(),
            "live_preview": self.live_preview,
            "dirty": self.buf.is_dirty(),
            "cursor": {"line": cursor.line, "col": cursor.col, "offset": self.buf.head},
            "selection": [sel.start, sel.end],
            "cursors": self.buf.cursor_count(),
            "scroll": {"line": self.scroll_line, "px": f32::from(self.scroll_px)},
            "visible_lines": [first, last],
            "line_count": self.buf.line_count(),
            "folds": self.folds.record(&self.foldables).iter().map(|(from, to)| serde_json::json!({"from": from, "to": to})).collect::<Vec<_>>(),
            "foldable_lines": self.foldables.iter().map(|f| f.line).collect::<Vec<_>>(),
            "field": self.field_kind(),
            "parsing": self.is_parsing(),
            "find": {
                "open": self.find.open,
                "replace": self.find.replace_shown,
                "query": self.find.query.read(cx).text(),
                "replacement": self.find.replacement.read(cx).text(),
                "matches": self.find.matches.len(),
                "current": self.find.current,
                "case": self.find.opts.match_case,
                "regex": self.find.opts.regex,
            },
            "suggest": self.describe_suggest(),
        });
        if text {
            out["text"] = serde_json::Value::String(self.buf.text().to_string());
        }
        out
    }

    /// The open autocompletion popup: what it is completing, the query it
    /// is completing, and the rows it is offering.
    ///
    /// The popup keeps only the rows and the columns accepting one
    /// replaces, so the kind and the query come from asking the trigger
    /// about those columns again -- the same call that opened it. A cursor
    /// that has since left the query answers nothing, and the markers it
    /// replaces name the kind instead.
    fn describe_suggest(&self) -> Option<serde_json::Value> {
        let s = self.suggest.as_ref()?;
        let line = if s.line < self.buf.line_count() {
            self.buf.line_text(s.line)
        } else {
            ""
        };
        let replaced = line.get(s.range.clone()).unwrap_or_default();
        let t = suggest::trigger(line, s.range.end.min(line.len()));
        let kind = match t.as_ref().map(|t| t.kind) {
            Some(suggest::Kind::Link) => "link",
            Some(suggest::Kind::Embed) => "embed",
            Some(suggest::Kind::Tag) => "tag",
            None if replaced.starts_with("![[") => "embed",
            None if replaced.starts_with("[[") => "link",
            None => "tag",
        };
        Some(serde_json::json!({
            "kind": kind,
            "query": t.map(|t| t.query).unwrap_or_default(),
            "selected": s.selected,
            "rows": s.items.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(),
        }))
    }

    /// Selects `start..end`, byte offsets clamped to the buffer, and
    /// reveals the cursor: the primary selection a drag over those bytes
    /// would have left, with the other cursors dropped.
    /// Puts the cursor at `offset` and reveals it on the next frame.
    ///
    /// Navigation that starts outside the editor — an outline row, a backlink,
    /// a search hit, a `[[Note#Heading]]` link — has no window at hand, so the
    /// scroll cannot be computed here; `pending_reveal` is the editor's own
    /// way of saying "put the cursor on screen when you next paint".
    pub fn reveal_offset(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.buf.collapse_cursors();
        let offset = self.buf.clamp_offset(offset);
        self.buf.anchor = offset;
        self.buf.head = offset;
        self.buf.goal = None;
        self.buf.marked = None;
        self.update_suggest(false, cx);
        self.pending_reveal = true;
        cx.notify();
    }

    /// [`Editor::reveal_offset`], with the cursor's row scrolled to the
    /// vertical centre of the view.
    pub fn reveal_offset_centered(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.reveal_offset(offset, cx);
        self.pending_center = true;
    }

    pub fn set_selection_range(
        &mut self,
        start: usize,
        end: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.buf.collapse_cursors();
        self.buf.anchor = self.buf.clamp_offset(start);
        self.buf.head = self.buf.clamp_offset(end);
        self.buf.goal = None;
        self.buf.marked = None;
        // * Like any other cursor move: refresh a popup that is open, and
        //   close it once the cursor has left its query.
        self.update_suggest(false, cx);
        self.reveal_cursor(window);
        cx.notify();
    }

    /// Whether a decoration pass is in flight: one running on the
    /// background thread, or one queued behind it for an edit that landed
    /// while it ran. Nothing waiting on the editor is settled until this
    /// is false.
    pub fn is_parsing(&self) -> bool {
        self.pending_parse.is_some() || self.parse_queued
    }

    /// Which of the find panel's two fields has the keys, when one does.
    /// Both sit in the same key context, so the focus path cannot tell them
    /// apart and the editor, which owns the handles, is asked instead.
    pub fn find_focus(&self, window: &Window, cx: &App) -> Option<&'static str> {
        if self.find.query.focus_handle(cx).is_focused(window) {
            Some("find-query")
        } else if self.find.replacement.focus_handle(cx).is_focused(window) {
            Some("find-replacement")
        } else {
            None
        }
    }

    /// The field the keys are going to while one is open: a property row's
    /// or a table cell's.
    pub fn field_kind(&self) -> Option<&'static str> {
        if self.property_field.is_some() {
            Some("property")
        } else if self.table_field.is_some() {
            Some("cell")
        } else {
            None
        }
    }

    /// First and last line painted in the last frame. Nothing is measured
    /// before the first paint, so the scroll anchor stands in for both.
    pub fn visible_line_range(&self) -> (usize, usize) {
        match (self.layout.visible.first(), self.layout.visible.last()) {
            (Some(first), Some(last)) => (first.line, last.line),
            _ => (self.scroll_line, self.scroll_line),
        }
    }

    /// The focus handle itself, so the workspace can tell whether the
    /// window's focus is this editor's without cloning it.
    pub fn focus_handle_ref(&self) -> &FocusHandle {
        &self.focus_handle
    }

    // ----- shaping outside a frame ----------------------------------------------

    /// The parts of the selections (and cursors) on `line`, line-relative;
    /// Live Preview reveals the source of the constructs they touch.
    fn touches_on_line(&self, line: usize) -> Vec<Range<usize>> {
        let range = self.buf.line_range(line);
        let mut out = Vec::new();
        let primary = Sel {
            anchor: self.buf.anchor,
            head: self.buf.head,
            goal: None,
        };
        for s in std::iter::once(&primary).chain(self.buf.extra.iter()) {
            let sel = s.range();
            if sel.end < range.start || sel.start > range.end {
                continue;
            }
            out.push(
                sel.start.max(range.start) - range.start..sel.end.min(range.end) - range.start,
            );
        }
        out
    }

    /// What `line` shows: its source, or Live Preview's rendering of it.
    fn display_for(&self, line: usize) -> DisplayLine {
        let text = self.buf.line_text(line);
        let display = if self.live_preview {
            let spans = self.highlights.get(line).map(Vec::as_slice).unwrap_or(&[]);
            // * An inline formula is shaped as a placeholder the width it
            //   typesets to, at the body size: a heading's scale is applied
            //   to the shaped line, placeholder included.
            let math = |tex: &str, block: bool| -> Option<String> {
                let (size, spaces) = self.spaces.as_ref()?;
                let formula = silica_math::typeset(tex, block).ok()?;
                Some(spaces.placeholder(crate::math::width_px(&formula, *size)).0)
            };
            preview::display_line_with(text, spans, &self.touches_on_line(line), &math)
        } else {
            DisplayLine::verbatim(text, 1.0, Default::default())
        };
        // * A folded line shows `…` after its text in both editing modes:
        //   raw source mode was observed folding exactly like Live Preview.
        if self.folds.is_folded(line) {
            return display.with_fold_placeholder(text.len());
        }
        display
    }

    // ----- editing -------------------------------------------------------------

    /// Replaces `range` with `insert` at the primary cursor as one undo
    /// step, then refreshes what the view keeps around the text.
    fn edit(
        &mut self,
        range: Range<usize>,
        insert: &str,
        kind: EditKind,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        self.buf.edit(range, insert, kind);
        self.after_edit(window, cx, None);
    }

    /// One replacement per selection as one undo step; see
    /// [`TextBuffer::edit_all`].
    fn edit_all(
        &mut self,
        edits: Vec<(Range<usize>, String)>,
        kind: EditKind,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        if self.buf.edit_all(edits, kind) {
            self.after_edit(window, cx, None);
        }
    }

    /// Inserts `s` at every cursor, replacing what each has selected.
    fn insert_all(&mut self, s: &str, kind: EditKind, window: &mut Window, cx: &mut Context<Self>) {
        if self.buf.insert_all(s, kind) {
            self.after_edit(Some(window), cx, None);
        }
    }

    /// One typed character: the pair it opens, closes, skips or wraps the
    /// selection in, or a plain insertion when it is none of those. Anything
    /// longer is not a keystroke and is inserted as it stands. True when the
    /// buffer changed.
    fn type_char_in(
        buf: &mut TextBuffer,
        s: &str,
        kind: EditKind,
        opts: autopair::Options,
    ) -> bool {
        let mut chars = s.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if buf.type_pair(c, opts) => true,
            _ => buf.insert_all(s, kind),
        }
    }

    fn type_char(&mut self, s: &str, kind: EditKind, window: &mut Window, cx: &mut Context<Self>) {
        let opts = self.auto_pair();
        if Self::type_char_in(&mut self.buf, s, kind, opts) {
            self.after_edit(Some(window), cx, None);
        }
    }

    /// Deletes at every cursor: the selection, or what `range` gives for a
    /// collapsed one.
    fn delete_all(
        &mut self,
        range: impl Fn(&TextBuffer, usize) -> Range<usize>,
        kind: EditKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.buf.delete_all(range, kind) {
            self.after_edit(Some(window), cx, None);
        }
    }

    fn after_edit(
        &mut self,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
        changed: Option<std::ops::Range<usize>>,
    ) {
        // * An edit never happens out of sight.
        self.unfold_at_cursor();
        if let Some(range) = changed {
            self.dirty_ranges.push(range);
        }
        self.request_parse(cx);
        self.refresh_matches(cx);
        self.update_suggest(true, cx);
        match window {
            Some(window) => self.reveal_cursor(window),
            None => self.pending_reveal = true,
        }
        cx.emit(EditorEvent::Edited);
        cx.notify();
    }

    /// Applies several non-overlapping replacements as one undo step and
    /// puts the cursor at `cursor` (a selection when `anchor` is given),
    /// both in post-edit coordinates; other cursors are dropped. Ranges may
    /// come in any order.
    fn edit_many(
        &mut self,
        edits: Vec<(Range<usize>, String)>,
        cursor: usize,
        anchor: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.buf.edit_many(edits, cursor, anchor) {
            self.after_edit(Some(window), cx, None);
        }
    }

    /// Recomputes decorations on the background executor. One parse runs at
    /// a time; edits during a parse queue exactly one more, so fast typing
    /// still sees decorations refresh at the parser's pace instead of
    /// cancelling every pass before it finishes.
    fn request_parse(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        if self.pending_parse.is_some() {
            self.parse_queued = true;
            return;
        }
        self.parse_queued = false;
        let generation = self.generation;
        let text = self.buf.text().to_string();
        let requested_at = Instant::now();
        let widget_source = self
            .live_preview
            .then(|| self.embed_source.clone())
            .flatten();
        let build_options = self.build_options;
        let live = self.live_preview;
        let path = self.name.to_string();
        let fold_options = self.fold_options();
        self.pending_parse = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let started = Instant::now();
                    let doc = silica_markdown::parse(&text);
                    let lines = highlight_lines(&doc);
                    let foldables = fold::foldables(&doc, &text, fold_options);
                    let widgets = match &widget_source {
                        Some(source) => {
                            let built = build_widgets(&path, &doc, source.as_ref(), build_options);
                            widget_lines(&text, built)
                        }
                        None => Vec::new(),
                    };
                    let flairs = match live {
                        true => code_flairs(&text, &doc),
                        false => Vec::new(),
                    };
                    let elapsed = started.elapsed();
                    let spans = lines.iter().map(Vec::len).sum();
                    let words = text.split_whitespace().count();
                    let chars = text.chars().count();
                    (
                        lines,
                        elapsed,
                        spans,
                        text.len(),
                        words,
                        chars,
                        widgets,
                        flairs,
                        foldables,
                    )
                })
                .await;
            this.update(cx, |editor, cx| {
                let parse_ms = result.1.as_secs_f64() * 1000.0;
                // * A slightly stale decoration set is better than none while typing.
                editor.highlights = Arc::new(result.0);
                let widgets = result.6;
                editor
                    .widget_heights
                    .retain(|first, _| widgets.iter().any(|(l, _)| l.start == *first));
                // * Widgets change the heights above the cursor: keep it on screen.
                let same_widgets = editor.widgets.len() == widgets.len()
                    && editor
                        .widgets
                        .iter()
                        .zip(&widgets)
                        .all(|((a, _), (b, _))| a == b);
                if !same_widgets {
                    editor.pending_reveal = true;
                }
                editor.widgets = Arc::new(widgets);
                editor.flairs = Arc::new(result.7);
                // * Unlike the decorations, a stale set of foldables is worse
                //   than none: resolving the folds against another text's
                //   lines would drop them. Only the parse of the current text
                //   settles them, and a record waiting for the first parse
                //   waits for that one.
                if editor.generation == generation {
                    editor.foldables = Arc::new(result.8);
                    if let Some(froms) = editor.pending_folds.take() {
                        editor.folds = fold::Folds::from_record(&editor.foldables, froms);
                    }
                    editor.refresh_folds();
                }
                editor.dirty_ranges.clear();
                editor.stats = ParseStats {
                    parse_ms,
                    spans: result.2,
                    bytes: result.3,
                    words: result.4,
                    chars: result.5,
                };
                editor.parse_samples.add(parse_ms);
                if editor.generation == generation {
                    editor
                        .parse_latency
                        .add(requested_at.elapsed().as_secs_f64() * 1000.0);
                }
                editor.pending_parse = None;
                if editor.parse_queued {
                    editor.request_parse(cx);
                }
                cx.notify();
            })
            .ok();
        }));
    }

    // ----- utf-16 helpers for the input handler (cursor line only) -----------

    fn utf16_to_byte(line: &str, utf16: usize) -> usize {
        let mut units = 0;
        for (i, c) in line.char_indices() {
            if units >= utf16 {
                return i;
            }
            units += c.len_utf16();
        }
        line.len()
    }

    fn byte_to_utf16(line: &str, byte: usize) -> usize {
        line[..byte.min(line.len())]
            .chars()
            .map(char::len_utf16)
            .sum()
    }

    /// A line-relative UTF-16 range on the cursor's line, as an absolute byte range.
    fn line_relative(&self, range_utf16: Range<usize>) -> Range<usize> {
        let c = self.buf.cursor_at(self.buf.head);
        let line_start = self.buf.line_starts()[c.line];
        let line = self.buf.line_text(c.line);
        line_start + Self::utf16_to_byte(line, range_utf16.start)
            ..line_start + Self::utf16_to_byte(line, range_utf16.end)
    }
}

/// Pairs each widget with the lines its span covers in `text`.
fn widget_lines(text: &str, widgets: Vec<Widget>) -> Vec<(Range<usize>, Widget)> {
    let starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let line_of = |offset: usize| match starts.binary_search(&offset) {
        Ok(i) => i,
        Err(i) => i - 1,
    };
    widgets
        .into_iter()
        .map(|w| {
            let first = line_of(w.span.start);
            let last = line_of(w.span.end.saturating_sub(1).max(w.span.start));
            (first..last + 1, w)
        })
        .collect()
}

/// Obsidian's `span.code-block-flair`: the label at the top right of a
/// fence's opening line in Live Preview, which is also that block's copy
/// button -- clicked, it copies the body and raises the "Copied to your
/// clipboard" notice the reading view's own button never raises. Source
/// mode draws none at all, and an indented code block, having no fence
/// line to carry one, gets none either: a note of eight blocks whose only
/// indented one was the fifth showed seven flairs. Observed 2026-10-25,
/// `docs/silica/Specification.md`.
#[derive(Debug, Clone, PartialEq)]
struct Flair {
    /// The line the opening fence is on.
    line: usize,
    /// The language's display name, the info string verbatim when there is
    /// no name for it, empty when the fence carries no info string.
    label: SharedString,
    /// What a click writes: the body with `\n` endings and exactly one
    /// trailing newline removed -- the payload the reading view's own
    /// button carries, by the same rule.
    copy: String,
}

/// The display name a flair shows for an info string. The table behind it
/// is Prism's, of which six names were read out of a running Obsidian;
/// anything else is shown verbatim, which is what an unregistered
/// `silica-made-up` did.
fn language_display_name(info: &str) -> SharedString {
    match info.trim().to_ascii_lowercase().as_str() {
        "js" => "JavaScript".into(),
        "ts" => "TypeScript".into(),
        "python" => "Python".into(),
        "rust" => "Rust".into(),
        "c" => "C".into(),
        "yaml" => "YAML".into(),
        "" => SharedString::default(),
        _ => SharedString::from(info.trim().to_string()),
    }
}

/// Every fenced code block in `doc`, as the flair its opening line carries.
/// Nested blocks count: a fence inside a blockquote or a list item has an
/// opening line of its own.
fn code_flairs(text: &str, doc: &Document<'_>) -> Vec<Flair> {
    let starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let line_of = |offset: usize| match starts.binary_search(&offset) {
        Ok(i) => i,
        Err(i) => i - 1,
    };
    let mut out = Vec::new();
    collect_flairs(&doc.blocks, &line_of, &mut out);
    out.sort_by_key(|f| f.line);
    out
}

fn collect_flairs(blocks: &[Block<'_>], line_of: &dyn Fn(usize) -> usize, out: &mut Vec<Flair>) {
    for block in blocks {
        match &block.kind {
            BlockKind::CodeBlock {
                fence: Some(_),
                info,
                content,
            } => {
                // * The same normalization the reading model does: Obsidian
                //   renders and copies with `\n` endings whatever the file
                //   holds, and drops exactly one trailing newline.
                let body = match content.contains('\r') {
                    true => content.replace("\r\n", "\n"),
                    false => content.to_string(),
                };
                out.push(Flair {
                    line: line_of(block.span.start),
                    label: language_display_name(info.as_deref().unwrap_or("")),
                    copy: body.strip_suffix('\n').unwrap_or(&body).to_string(),
                });
            }
            BlockKind::CodeBlock { .. } => {}
            BlockKind::BlockQuote { children } | BlockKind::Callout { children, .. } => {
                collect_flairs(children, line_of, out)
            }
            BlockKind::List { items, .. } => {
                for item in items {
                    collect_flairs(&item.children, line_of, out);
                }
            }
            _ => {}
        }
    }
}

impl Focusable for Editor {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EntityInputHandler for Editor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let line = self.buf.line_text(self.cursor().line);
        let start = Self::utf16_to_byte(line, range_utf16.start);
        let end = Self::utf16_to_byte(line, range_utf16.end);
        adjusted_range.replace(Self::byte_to_utf16(line, start)..Self::byte_to_utf16(line, end));
        Some(line[start..end].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let c = self.cursor();
        let range = self.buf.line_range(c.line);
        let line = &self.buf.text()[range.clone()];
        let sel = self.selection();
        let (start, end) = if sel.start >= range.start && sel.end <= range.end {
            (sel.start - range.start, sel.end - range.start)
        } else {
            (c.col, c.col)
        };
        Some(UTF16Selection {
            range: Self::byte_to_utf16(line, start)..Self::byte_to_utf16(line, end),
            reversed: self.buf.head < self.buf.anchor,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        let c = self.cursor();
        let range = self.buf.line_range(c.line);
        let marked = self.buf.marked.clone()?;
        if marked.start < range.start || marked.end > range.end {
            return None;
        }
        let line = &self.buf.text()[range.clone()];
        Some(
            Self::byte_to_utf16(line, marked.start - range.start)
                ..Self::byte_to_utf16(line, marked.end - range.start),
        )
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.buf.marked = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // * A properties field takes the keystrokes while it is open; the
        //   document changes once, when the field commits.
        if let Some(field) = self.active_field() {
            field.insert(new_text);
            cx.notify();
            return;
        }
        let kind = if new_text.contains('\n') {
            EditKind::Other
        } else {
            EditKind::Typing
        };
        let range = range_utf16
            .map(|r| self.line_relative(r))
            .or_else(|| self.buf.marked.clone());
        match range {
            // * A ranged replacement (IME) addresses the primary cursor's line.
            Some(range) => self.edit(range, new_text, kind, Some(window), cx),
            // * Auto-pairing is a typed character only: a paste has its own
            //   path and an IME commit arrives as a range above.
            None => self.type_char(new_text, kind, window, cx),
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .map(|r| self.line_relative(r))
            .or_else(|| self.buf.marked.clone())
            .unwrap_or_else(|| self.selection());
        self.edit(range.clone(), new_text, EditKind::Typing, Some(window), cx);
        let marked = range.start..range.start + new_text.len();
        if let Some(sel) = new_selected_range {
            let at = marked.start + Self::utf16_to_byte(new_text, sel.end);
            self.buf.head = self.buf.clamp_offset(at);
            self.buf.anchor = self.buf.head;
        }
        self.buf.marked = Some(marked);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let shaped = self.layout.cursor_line.clone()?;
        let line = self.buf.line_text(self.cursor().line);
        let start = Self::utf16_to_byte(line, range_utf16.start);
        let end = Self::utf16_to_byte(line, range_utf16.end);
        let rh = shaped.row_height;
        let (row, x0) = shaped.row_and_x(start);
        let (_, x1) = shaped.row_and_x(end);
        let y = self.layout.cursor_top + rh * row as f32;
        let x = self.layout.content_x;
        Some(Bounds::from_corners(
            point(x + x0, y),
            point(x + x1.max(x0), y + rh),
        ))
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl Render for Editor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        self.palette = Palette::from_theme(&theme);
        let base = theme.base_font_size;
        let panel = self.find.open.then(|| self.render_find_panel(&theme, cx));
        // * The popup is drawn over the text, anchored under the query it is
        //   offering rows for.
        let popup = self.suggest.take().map(|state| {
            let element = self
                .suggest_anchor(&state)
                .map(|anchor| crate::suggest::render(&state, &anchor, &theme, cx));
            self.suggest = Some(state);
            element
        });
        let editor = div()
            // * A second identifier while the popup is up: its own bindings
            //   match deeper than the workspace's and so shadow them.
            .key_context(if self.suggest.is_some() {
                "Editor SuggestPopup"
            } else {
                "Editor"
            })
            .track_focus(&self.focus_handle)
            .flex_1()
            .font_family(theme.text_font.clone())
            .text_size(px(base))
            .line_height(px(base * 1.5))
            .text_color(theme.text)
            .on_action(cx.listener(Self::move_left))
            .on_action(cx.listener(Self::move_right))
            .on_action(cx.listener(Self::move_up))
            .on_action(cx.listener(Self::move_down))
            .on_action(cx.listener(Self::move_word_left))
            .on_action(cx.listener(Self::move_word_right))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::line_start))
            .on_action(cx.listener(Self::line_end))
            .on_action(cx.listener(Self::document_start))
            .on_action(cx.listener(Self::document_end))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::select_line_start))
            .on_action(cx.listener(Self::select_line_end))
            .on_action(cx.listener(Self::select_page_up))
            .on_action(cx.listener(Self::select_page_down))
            .on_action(cx.listener(Self::select_document_start))
            .on_action(cx.listener(Self::select_document_end))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::delete_word_right))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::newline_plain))
            .on_action(cx.listener(Self::tab))
            .on_action(cx.listener(Self::outdent))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::close_find))
            .on_action(cx.listener(Self::add_cursor_above))
            .on_action(cx.listener(Self::add_cursor_below))
            .child(EditorElement {
                editor: cx.entity(),
            });
        let preview = self
            .hover_preview
            .as_ref()
            .map(|(pos, data)| crate::page_preview::render_floating(data, *pos, &theme));
        // * The attachment toolbar rides above the text, and only with a
        //   note open: there is nothing to attach to an empty pane.
        let toolbar = (!self.name.is_empty()).then(|| self.render_attach_toolbar(&theme, cx));
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .on_drop(cx.listener(Self::handle_drop))
            .children(toolbar)
            .child(editor)
            .children(panel)
            .children(popup.flatten())
            .children(preview)
            .children(self.render_menu(cx))
    }
}

impl Editor {
    /// Obsidian's search panel: a floating box at the top right of the
    /// editor with the query, previous and next, the match count, and the
    /// replace row with its field and buttons.
    fn render_find_panel(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.find.matches.len();
        let query_empty = self.find.query.read(cx).text().is_empty();
        let status: SharedString = match (query_empty, self.find.current) {
            (true, _) => String::new(),
            (false, Some(ix)) => format!("{} of {count}", ix + 1),
            (false, None) if count == 0 => "No results".to_string(),
            (false, None) => format!("{count} results"),
        }
        .into();
        let replace_shown = self.find.replace_shown;
        let opts = self.find.opts;
        let button = |id: &'static str, label: &'static str, theme: &Theme| {
            div()
                .id(id)
                .px_2()
                .py_px()
                .rounded_sm()
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(theme.text)
                .bg(theme.bg_alt)
                .hover(|s| s.bg(theme.bg_active))
                .child(label)
        };
        // * A toggle shows its state with the accent color.
        let toggle = |id: &'static str, label: &'static str, on: bool, theme: &Theme| {
            let mut b = button(id, label, theme).border_1();
            b = if on {
                b.border_color(theme.accent).text_color(theme.accent)
            } else {
                b.border_color(theme.bg_alt)
            };
            b
        };
        let find_row =
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_1()
                .child(div().w(px(220.)).child(self.find.query.clone()))
                .child(
                    toggle("find-match-case", "Aa", opts.match_case, theme)
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_find_option(false, cx))),
                )
                .child(
                    toggle("find-regex", ".*", opts.regex, theme)
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_find_option(true, cx))),
                )
                .child(
                    button("find-prev", "↑", theme)
                        .on_click(cx.listener(|this, _, _, cx| this.find_step(false, cx))),
                )
                .child(
                    button("find-next", "↓", theme)
                        .on_click(cx.listener(|this, _, _, cx| this.find_step(true, cx))),
                )
                .child(button("find-all", "Find all", theme).on_click(
                    cx.listener(|this, _, window, cx| this.find_all(&FindAll, window, cx)),
                ))
                .child(
                    div()
                        .min_w(px(70.))
                        .text_size(px(11.))
                        .text_color(theme.text_muted)
                        .child(status),
                )
                .child(
                    button(
                        "find-toggle-replace",
                        if replace_shown {
                            "Replace ▴"
                        } else {
                            "Replace ▾"
                        },
                        theme,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.find.replace_shown = !this.find.replace_shown;
                        cx.notify();
                    })),
                )
                .child(button("find-close", "×", theme).on_click(
                    cx.listener(|this, _, window, cx| this.close_find(&CloseFind, window, cx)),
                ));
        let replace_row = replace_shown.then(|| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_1()
                .child(div().w(px(220.)).child(self.find.replacement.clone()))
                .child(
                    button("replace-one", "Replace", theme)
                        .on_click(cx.listener(|this, _, _, cx| this.replace_next(cx))),
                )
                .child(
                    button("replace-all", "Replace all", theme)
                        .on_click(cx.listener(|this, _, _, cx| this.replace_all(cx))),
                )
        });
        div()
            .id("find-panel")
            .key_context("FindPanel")
            .occlude()
            .absolute()
            .top(px(8.))
            .right(px(24.))
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .rounded_md()
            .bg(theme.bg_secondary)
            .border_1()
            .border_color(theme.border)
            .font_family(theme.ui_font.clone())
            .on_action(cx.listener(Self::find_previous))
            .on_action(cx.listener(Self::find_next))
            .on_action(cx.listener(Self::find_all))
            .on_action(cx.listener(Self::close_find))
            .on_action(cx.listener(Self::focus_other_field))
            .child(find_row)
            .children(replace_row)
    }

    /// The editor's right-click context menu items, matching Obsidian's.
    fn menu_items(_target: Option<usize>) -> Vec<MenuItem> {
        use EditorEvent as E;
        let item = |label, icon, event| MenuItem { label, icon, event };
        let mut items: Vec<MenuItem> = vec![
            item(t("Cut"), "scissors", E::Cut),
            item(t("Copy"), "copy", E::Copy),
            item(t("Paste"), "clipboard", E::Paste),
        ];
        items.push(item(t("Select all"), "select-all", E::SelectAll));
        items.push(item(t("Toggle bold"), "bold", E::ToggleBold));
        items.push(item(t("Toggle italic"), "italic", E::ToggleItalic));
        items.push(item(
            t("Toggle highlight"),
            "highlighter",
            E::ToggleHighlight,
        ));
        items.push(item(t("Insert link"), "link", E::InsertLink));
        items.push(item(t("Insert wikilink"), "link-2", E::InsertWikilink));
        items.push(item(t("Insert embed"), "image", E::InsertEmbed));
        items.push(item(
            t("Toggle checkbox"),
            "check-square",
            E::ToggleCheckbox,
        ));
        items.push(item(t("Heading 1"), "type", E::SetHeading1));
        items.push(item(t("Heading 2"), "type", E::SetHeading2));
        items.push(item(t("Bullet list"), "list", E::ToggleBullet));
        items.push(item(t("Numbered list"), "list-ordered", E::ToggleNumbered));
        items.push(item(t("Blockquote"), "quote", E::ToggleBlockquote));
        items
    }

    /// Renders the right-click context menu over the editor.
    fn render_menu(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let menu = self.context_menu.as_ref()?;
        let theme = cx.global::<Theme>().clone();
        let items = Self::menu_items(menu.target);
        let mut list = div()
            .id("editor-menu")
            .absolute()
            .absolute()
            .left(menu.position.x)
            .top(menu.position.y)
            .flex()
            .flex_col()
            .min_w(px(180.))
            .py_1()
            .bg(theme.bg_secondary)
            .border_1()
            .border_color(theme.border)
            .rounded_md()
            .shadow_lg()
            .text_size(px(13.))
            .text_color(theme.text)
            .occlude();
        for item in items {
            let entity = cx.entity();
            list = list.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_0p5()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.bg_active))
                    .child(
                        div()
                            .w(px(16.))
                            .flex_shrink_0()
                            .text_color(theme.text_muted)
                            .child(crate::assets::icon(item.icon, 14., theme.text_muted)),
                    )
                    .child(item.label)
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(move |_this, _, _, cx| {
                            entity.update(cx, |e, cx| {
                                e.context_menu = None;
                                match item.event {
                                    EditorEvent::Cut => e.do_cut(None, cx),
                                    EditorEvent::Copy => e.do_copy(None, cx),
                                    EditorEvent::Paste => e.do_paste(None, cx),
                                    EditorEvent::SelectAll => e.do_select_all(cx),
                                    EditorEvent::ToggleBold => e.toggle_wrap("**", None, cx),
                                    EditorEvent::ToggleItalic => e.toggle_wrap("*", None, cx),
                                    EditorEvent::ToggleHighlight => e.toggle_wrap("==", None, cx),
                                    EditorEvent::InsertWikilink => {
                                        // * Handled by typing [[ in the editor
                                    }
                                    EditorEvent::InsertLink => {
                                        // * Handled by typing [](url) in the editor
                                    }
                                    EditorEvent::InsertEmbed => {
                                        // * Handled by typing ![[ in the editor
                                    }
                                    EditorEvent::ToggleCheckbox => e.toggle_checklist(None, cx),
                                    EditorEvent::SetHeading1 => e.set_heading(1, None, cx),
                                    EditorEvent::SetHeading2 => e.set_heading(2, None, cx),
                                    EditorEvent::ToggleBullet => e.toggle_list(None, cx),
                                    EditorEvent::ToggleNumbered => e.toggle_list(None, cx),
                                    EditorEvent::ToggleBlockquote => e.toggle_blockquote(None, cx),
                                    _ => {}
                                }
                            });
                        }),
                    ),
            );
        }
        Some(list.into_any_element())
    }
}

// * Element

/// Styles one flattened highlight segment on top of the base text style.
/// `done` marks a checked task's line, whose text is muted and struck
/// through; the checkbox glyph keeps its own colour and is left unstruck,
/// since it stands in for Obsidian's `<input>` rather than for text.
fn run_for(
    kinds: &[HighlightKind],
    base: &TextStyle,
    len: usize,
    p: &Palette,
    done: bool,
) -> TextRun {
    let mut font = base.font();
    let mut color: Hsla = base.color;
    let mut background_color = None;
    let mut underline = None;
    let mut strikethrough = None;
    for kind in kinds {
        match kind {
            HighlightKind::Heading(h) => {
                // * The setext underline row is the heading's markup: it keeps
                //   the heading colour and its own line, but body weight and
                //   body size (`line_block`), which is what Obsidian computes
                //   for `cm-formatting-header` — 16 px at 400.
                if !h.underline {
                    font.weight = FontWeight::BOLD;
                }
                color = p.accent;
            }
            HighlightKind::Wikilink | HighlightKind::Embed | HighlightKind::Link => {
                color = p.link;
                underline = Some(UnderlineStyle {
                    color: Some(p.link),
                    thickness: px(1.),
                    wavy: false,
                });
            }
            HighlightKind::Url | HighlightKind::Footnote => color = p.link,
            HighlightKind::Tag => color = p.tag,
            HighlightKind::Code | HighlightKind::CodeBlock => {
                font = Font {
                    family: p.mono.clone(),
                    ..font
                };
                background_color = Some(p.code_bg);
            }
            HighlightKind::Emphasis => font.style = FontStyle::Italic,
            HighlightKind::Strong => font.weight = FontWeight::BOLD,
            HighlightKind::Strikethrough => {
                strikethrough = Some(StrikethroughStyle {
                    thickness: px(1.),
                    color: Some(color),
                });
            }
            HighlightKind::Highlight => background_color = Some(p.highlight_bg),
            HighlightKind::Comment
            | HighlightKind::Frontmatter
            | HighlightKind::BlockId
            | HighlightKind::ThematicBreak
            | HighlightKind::QuoteMarker
            | HighlightKind::Html => color = p.muted,
            HighlightKind::Math => color = p.accent,
            HighlightKind::Callout | HighlightKind::ListMarker | HighlightKind::Task => {
                color = p.accent;
            }
        }
    }
    if done && !kinds.contains(&HighlightKind::Task) {
        color = p.muted;
        strikethrough = Some(StrikethroughStyle {
            thickness: px(1.),
            color: Some(color),
        });
    }
    TextRun {
        len,
        font,
        color,
        background_color,
        underline,
        strikethrough,
    }
}

/// Builds runs covering the display text: the source's highlighted
/// segments carried through copied pieces, a substitution styled as the
/// construct it stands for.
fn runs_for_display(
    display: &DisplayLine,
    spans: &[HighlightSpan],
    base: &TextStyle,
    p: &Palette,
) -> Vec<TextRun> {
    let flat = flatten(spans);
    let done = display.decor.task_done;
    let mut runs: Vec<TextRun> = Vec::new();
    let mut push = |run: TextRun| {
        if run.len > 0 {
            runs.push(run);
        }
    };
    // * The stretches no construct covers take the same treatment, which on a
    //   done task is most of the line.
    let plain = |len: usize| run_for(&[], base, len, p, done);
    for piece in &display.pieces {
        match piece {
            Piece::Copy { source, len, .. } => {
                let (s0, s1) = (*source, source + len);
                let mut at = s0;
                for (range, kinds) in &flat {
                    let start = range.start.max(s0);
                    let end = range.end.min(s1);
                    if end <= start || start < at {
                        continue;
                    }
                    if start > at {
                        push(plain(start - at));
                    }
                    push(run_for(kinds, base, end - start, p, done));
                    at = end;
                }
                if at < s1 {
                    push(plain(s1 - at));
                }
            }
            Piece::Sub {
                display: at,
                source,
                len,
                kind,
            } => {
                if *len == 0 {
                    continue;
                }
                let kinds = flat
                    .iter()
                    .find(|(r, _)| r.start <= source.start && source.start < r.end)
                    .map(|(_, k)| k.clone())
                    .unwrap_or_else(|| vec![*kind]);
                // * An inline HTML element drawn as itself: the constructs
                //   around it (a heading, a `**strong**` it sits in) and then
                //   its own tags' styles, never the muted colour of its
                //   source.
                let element = (*kind == HighlightKind::Html)
                    .then(|| display.html.iter().find(|h| h.display.start == *at))
                    .flatten();
                match element {
                    Some(element) => {
                        let outer: Vec<HighlightKind> = kinds
                            .into_iter()
                            .filter(|k| *k != HighlightKind::Html)
                            .collect();
                        let mut cut = 0;
                        let mut emit = |to: usize, style: InlineStyle| {
                            if to > cut {
                                let mut run = run_for(&outer, base, to - cut, p, done);
                                style_html_run(&mut run, style, p);
                                push(run);
                                cut = to;
                            }
                        };
                        for (range, style) in &element.runs {
                            emit(range.start, InlineStyle::default());
                            emit(range.end, *style);
                        }
                        emit(*len, InlineStyle::default());
                    }
                    None => push(run_for(&kinds, base, *len, p, done)),
                }
            }
        }
    }
    if runs.is_empty() {
        runs.push(base.to_run(0));
    }
    if done {
        // The gap between a done task's box and its text is not struck: a
        // struck space reads as a stray `-` before the words.
        for task in &display.tasks {
            let rest = &display.text[task.display.end..];
            let gap = rest.len() - rest.trim_start_matches([' ', '\t']).len();
            unstrike(&mut runs, task.display.end..task.display.end + gap);
        }
    }
    runs
}

/// Clears the strikethrough of the display bytes `range` in `runs`,
/// splitting a run that straddles either end.
fn unstrike(runs: &mut Vec<TextRun>, range: Range<usize>) {
    if range.is_empty() {
        return;
    }
    let mut out = Vec::with_capacity(runs.len() + 2);
    let mut at = 0;
    for run in runs.drain(..) {
        let (start, end) = (at, at + run.len);
        at = end;
        let (a, b) = (range.start.clamp(start, end), range.end.clamp(start, end));
        if a == b || run.strikethrough.is_none() {
            out.push(run);
            continue;
        }
        for (from, to, strike) in [(start, a, true), (a, b, false), (b, end, true)] {
            if to > from {
                let mut piece = run.clone();
                piece.len = to - from;
                if !strike {
                    piece.strikethrough = None;
                }
                out.push(piece);
            }
        }
    }
    *runs = out;
}

/// Lays an inline HTML element's own style over its run, the way the
/// reading view draws the same tags: `<code>` and `<kbd>` wear inline
/// code's mono and box, `<u>` underlines in the text colour, a raw
/// `<a href>` is a link. `<sup>` and `<sub>` have no look of their own,
/// as in the reading view.
fn style_html_run(run: &mut TextRun, s: InlineStyle, p: &Palette) {
    if s.bold {
        run.font.weight = FontWeight::BOLD;
    }
    if s.italic {
        run.font.style = FontStyle::Italic;
    }
    if s.code {
        run.font = Font {
            family: p.mono.clone(),
            ..run.font.clone()
        };
        run.background_color = Some(p.code_bg);
    }
    if s.highlight {
        run.background_color = Some(p.highlight_bg);
    }
    if s.strike {
        run.strikethrough = Some(StrikethroughStyle {
            thickness: px(1.),
            color: Some(run.color),
        });
    }
    if s.underline {
        run.underline = Some(UnderlineStyle {
            color: Some(run.color),
            thickness: px(1.),
            wavy: false,
        });
    }
    if s.link {
        run.color = p.link;
        run.underline = Some(UnderlineStyle {
            color: Some(p.link),
            thickness: px(1.),
            wavy: false,
        });
    }
}

/// Paints each task's box over its placeholder in `shaped`, whose first row
/// starts at `origin`: a rounded square of about an em, at the left of the
/// placeholder and centred on the row, outlined while open and filled with
/// a drawn tick when checked -- the reading view's box, from the same
/// tokens, and nothing a fallback font can turn into an emoji.
fn paint_task_boxes(
    shaped: &Shaped,
    origin: gpui::Point<Pixels>,
    em: Pixels,
    p: &Palette,
    window: &mut Window,
) {
    let rh = shaped.row_height;
    for task in &shaped.display.tasks {
        let Some(start) = shaped.line.position_for_index(task.display.start, rh) else {
            continue;
        };
        let room = shaped
            .line
            .position_for_index(task.display.end, rh)
            .filter(|end| end.y == start.y)
            .map(|end| end.x - start.x)
            .unwrap_or(em);
        let side = (em * 0.9).min(room).max(px(8.));
        let top = origin.y + start.y + (rh - side) / 2.;
        let left = origin.x + start.x + px(1.);
        let bounds = gpui::Bounds::new(gpui::point(left, top), size(side, side));
        let quad = if task.checked {
            fill(bounds, p.checkbox).border_color(p.checkbox)
        } else {
            fill(bounds, gpui::transparent_black()).border_color(p.checkbox_border)
        };
        window.paint_quad(quad.border_widths(px(1.)).corner_radii(px(3.)));
        if task.checked {
            let at = |x: f32, y: f32| gpui::point(left + side * x, top + side * y);
            let mut tick = gpui::PathBuilder::stroke((side * 0.12).max(px(1.5)));
            tick.move_to(at(0.25, 0.52));
            tick.line_to(at(0.43, 0.70));
            tick.line_to(at(0.76, 0.32));
            if let Ok(path) = tick.build() {
                window.paint_path(path, p.checkbox_marker);
            }
        }
    }
}

/// Shapes a display line at the editor's font, scaled for a heading.
#[allow(clippy::too_many_arguments)]
fn shape_display(
    window: &mut Window,
    style: &TextStyle,
    font_size: Pixels,
    line_height: Pixels,
    wrap_width: Option<Pixels>,
    display: DisplayLine,
    spans: &[HighlightSpan],
    p: &Palette,
) -> Shaped {
    let runs = runs_for_display(&display, spans, style, p);
    let size = font_size * display.scale;
    // * Headings keep Obsidian's tighter line height, never below the body's.
    let row_height = if display.scale > 1.0 {
        (size * 1.2).max(line_height)
    } else {
        line_height
    };
    let line = window
        .text_system()
        .shape_text(
            SharedString::from(display.text.clone()),
            size,
            &runs,
            wrap_width,
            None,
        )
        .ok()
        .and_then(|lines| lines.into_iter().next())
        .unwrap_or_default();
    // * A line with a tall inline formula (a fraction, a sum with limits)
    //   grows to hold it, the way CodeMirror's line grows around an inline
    //   widget; every wrapped row of the line shares the height.
    let mut row_height = row_height;
    if !display.math.is_empty() {
        let layout = &line.unwrapped_layout;
        let mut extra = px(0.);
        for m in &display.math {
            if let Ok(formula) = silica_math::typeset(&m.tex, m.block) {
                extra = extra.max(crate::math::inline_overflow(
                    &formula,
                    size,
                    layout.ascent,
                    layout.descent,
                ));
            }
        }
        row_height += extra;
    }
    Shaped {
        display,
        line,
        row_height,
    }
}

struct EditorElement {
    editor: Entity<Editor>,
}

/// A widget laid out for this frame, ready to paint.
struct WidgetPaint {
    lines: Range<usize>,
    visible: VisibleLine,
    bounds: Bounds<Pixels>,
    embed: bool,
    element: AnyElement,
}

struct Prepaint {
    lines: Vec<(VisibleLine, Shaped)>,
    /// What the mouse listeners ask whether the pointer is really on the
    /// text and not on something painted over it.
    hitbox: Hitbox,
    widgets: Vec<WidgetPaint>,
    /// Live Preview's code-block flairs, painted over the lines.
    flairs: Vec<AnyElement>,
    /// Quote bars, code block bands and rules, behind the text.
    decor: Vec<PaintQuad>,
    line_height: Pixels,
    font_size: Pixels,
    text_style: TextStyle,
    wrap_width: Option<Pixels>,
    content_x: Pixels,
    /// Width of the line-number gutter when `show_line_number`; zero otherwise.
    line_number_gutter: Pixels,
    /// Vertical indent-guide segments: `(x, top, height)`, painted when
    /// `show_indent_guide` is on. One segment per tab level per indented
    /// visible line.
    indent_guides: Vec<(Pixels, Pixels, Pixels)>,
    /// Misspelled-word squiggly underlines: `(x, top, width)`,
    /// painted when `spellcheck` is on. One segment per misspelled word.
    spellcheck_words: Vec<(Pixels, Pixels, Pixels)>,
    selection: Vec<PaintQuad>,
    cursors: Vec<PaintQuad>,
    cursor_line: Option<Arc<Shaped>>,
    cursor_top: Pixels,
    /// `scroll_px` was past the anchor line's rows; the clamped value.
    scroll_px: Pixels,
    scrollbar: Option<Scrollbar>,
    thumb_dragging: bool,
    prepaint_ms: f64,
}

impl IntoElement for EditorElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let started = Instant::now();
        // * The listeners this element registers are raw window listeners,
        //   which fire for any point inside its bounds; a panel over the text
        //   — the find panel, a modal — owns its own clicks, and this hitbox
        //   is how a listener asks whether one of those took the point first.
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let text_style = window.text_style();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let lh = window.line_height();
        let theme = cx.global::<Theme>().clone();
        let entity = self.editor.clone();

        // * The editor is leased for the frame so widgets (GPUI elements
        //   built from the reading renderer) can be laid out with the app
        //   at hand while its lines are read.
        self.editor.update(cx, |editor, cx| {
            // * Measured before any line is shaped, so this frame's inline
            //   formulas already have placeholders.
            editor.spaces = Some((
                font_size,
                SpaceWidths::measure(window.text_system(), &text_style.font(), font_size),
            ));
            // * A cursor sent into a fold from outside (a link, a search hit,
            //   an outline row) opens it before anything is laid out.
            if editor.pending_reveal {
                editor.unfold_at_cursor();
            }
            let palette = editor.palette.clone();
            let highlights = editor.highlights.clone();
            let flairs = editor.flairs.clone();
            let count = editor.line_count();
            let scroll_line = editor.scroll_line.min(count - 1);
            let mut scroll_px = editor.scroll_px;
            let cursors = editor.buf.cursors();
            let head = editor.buf.head;
            let focused = editor.focus_handle.is_focused(window);
            let thumb_dragging = editor.thumb_drag.is_some();

            let available = (bounds.size.width - px(GUTTER * 2.)).max(px(50.));
            let content_width = if editor.readable_line_length {
                available.min(px(READABLE_WIDTH))
            } else {
                available
            };
            // * Reserve a gutter for line numbers when enabled; shift content
            //   right so line numbers sit in the left margin.
            let gutter = if editor.show_line_number && count > 0 {
                let digits = count.to_string().len().max(1);
                px(16.) + digits as f32 * font_size * 0.6
            } else {
                px(0.)
            };
            let content_x = bounds.origin.x + (bounds.size.width - content_width) / 2. + gutter;
            let wrap_width = Some(content_width);

            let live = editor.live_preview;
            let matches: &[Range<usize>] = if editor.find.open {
                &editor.find.matches
            } else {
                &[]
            };
            let current_match = editor.find.current;
            let on_link: LinkHandler = Rc::new({
                let entity = entity.clone();
                move |target, _, cx| {
                    entity.update(cx, |_, cx| cx.emit(EditorEvent::OpenLink(target)));
                }
            });
            let on_toggle: ToggleHandler = Rc::new({
                let entity = entity.clone();
                move |id, cx| {
                    entity.update(cx, |e, cx| {
                        if id == crate::properties::PROPERTIES_FOLD_ID {
                            e.properties_folded = !e.properties_folded;
                        } else if !e.callout_folds.remove(&id) {
                            e.callout_folds.insert(id);
                        }
                        cx.notify();
                    });
                }
            });
            // * The properties block's fold state travels in the toggled set,
            //   the same place the reading view keeps callout folds.
            let mut no_toggles = editor.callout_folds.clone();
            if editor.properties_folded {
                no_toggles.insert(crate::properties::PROPERTIES_FOLD_ID);
            }
            let property_types = editor.property_types.clone();
            // * Live Preview's properties block is editable: the open field is
            //   drawn with its caret, and clicks come back as actions.
            let properties_host = PropertiesHost {
                field: editor.property_field.clone(),
                selected_pill: editor.selected_pill,
                mode: editor.properties_in_document,
                menu: editor.property_menu.clone(),
                on_action: Rc::new({
                    let entity = entity.clone();
                    move |action, window, cx| {
                        entity.update(cx, |e, cx| e.property_action(action, window, cx));
                    }
                }),
            };
            let table_host = TableHost {
                cell: editor
                    .table_field
                    .as_ref()
                    .map(|c| (c.span.clone(), c.field.clone())),
                on_action: Rc::new({
                    let entity = entity.clone();
                    move |action, window, cx| {
                        entity.update(cx, |e, cx| e.table_action(action, window, cx));
                    }
                }),
            };
            let renderer = Renderer::new(
                &theme,
                window.text_system().clone(),
                f32::from(content_width),
                &no_toggles,
                &property_types,
                Some(&properties_host),
                Some(&table_host),
                on_link,
                on_toggle,
                String::new(),
                WeakEntity::<ReadingView>::new_invalid(),
                None,
            );
            let mut lines: Vec<(VisibleLine, Shaped)> = Vec::new();
            let mut widgets: Vec<WidgetPaint> = Vec::new();
            let mut flair_elements: Vec<AnyElement> = Vec::new();
            let mut decor_quads = Vec::new();
            let mut selection_quads = Vec::new();
            let mut cursor_quads = Vec::new();
            let mut cursor_top = px(0.);
            let mut y = bounds.origin.y - scroll_px;
            let mut line = scroll_line;
            let hidden = editor.hidden.clone();
            let foldables = editor.foldables.clone();
            let hover_line = editor.hover_line;
            while line < count && y < bounds.bottom_left().y {
                if let Some(end) = fold::hidden_end(&hidden, line) {
                    // * Inside a fold: these lines are not on screen at all.
                    line = end + 1;
                    continue;
                }
                if let Some((wlines, widget)) = editor.folded_widget(line) {
                    let wlines = wlines.clone();
                    if line != wlines.start || editor.widget_is_hidden(widget) {
                        // * Scrolled into a folded widget: its lines take no
                        //   room. A hidden properties block takes none at all.
                        line = wlines.end;
                        continue;
                    }
                    // * An embed keeps its clicks so its links work, and a
                    //   table keeps them because its cells take their own:
                    //   neither ever unfolds to its source.
                    let embed = matches!(widget.block, RBlock::Paragraph(_) | RBlock::Table { .. });
                    let mut element = div()
                        .w(content_width)
                        .child(renderer.block(&widget.block, false))
                        .into_any_element();
                    let measured = element.layout_as_root(
                        size(
                            AvailableSpace::Definite(content_width),
                            AvailableSpace::MinContent,
                        ),
                        window,
                        cx,
                    );
                    let height = measured.height.max(lh);
                    if line == scroll_line && scroll_px >= height {
                        scroll_px = px(0.);
                        y = bounds.origin.y;
                    }
                    element.prepaint_at(point(content_x, y), window, cx);
                    widgets.push(WidgetPaint {
                        lines: wlines.clone(),
                        visible: VisibleLine {
                            line,
                            top: y - bounds.origin.y,
                            rows: 1,
                            row_height: height,
                        },
                        bounds: Bounds::new(point(content_x, y), size(content_width, height)),
                        embed,
                        element,
                    });
                    y += height;
                    line = wlines.end;
                    continue;
                }
                let spans = highlights.get(line).map(Vec::as_slice).unwrap_or(&[]);
                let shaped = shape_display(
                    window,
                    &text_style,
                    font_size,
                    lh,
                    wrap_width,
                    editor.display_for(line),
                    spans,
                    &palette,
                );
                let rows = shaped.rows();
                let rh = shaped.row_height;
                let height = shaped.height();
                if line == scroll_line && scroll_px >= height {
                    scroll_px = (height - rh).max(px(0.));
                    y = bounds.origin.y - scroll_px;
                }
                if live {
                    let d = shaped.display.decor;
                    if d.code_block || d.frontmatter {
                        decor_quads.push(fill(
                            Bounds::new(
                                point(content_x - px(8.), y),
                                size(content_width + px(16.), height),
                            ),
                            palette.code_bg,
                        ));
                    }
                    for level in 0..d.quote_depth {
                        let x = content_x - px(10.) * (d.quote_depth - level) as f32;
                        let mut bar = palette.muted;
                        bar.a = 0.6;
                        decor_quads.push(fill(Bounds::new(point(x, y), size(px(2.), height)), bar));
                    }
                    if d.rule {
                        decor_quads.push(fill(
                            Bounds::new(point(content_x, y + rh / 2.), size(content_width, px(1.))),
                            palette.border,
                        ));
                    }
                    if let Ok(ix) = flairs.binary_search_by_key(&line, |f| f.line) {
                        let flair = &flairs[ix];
                        let label = flair.label.clone();
                        let payload = flair.copy.clone();
                        let entity = entity.clone();
                        let mut element = div()
                            .id(("code-flair", line))
                            .px(px(8.))
                            .py(px(4.))
                            .rounded(px(4.))
                            .font_family(theme.ui_font.clone())
                            .text_size(px(theme.base_font_size * 0.8))
                            .text_color(palette.muted)
                            .cursor(CursorStyle::Arrow)
                            .hover(|s| s.bg(theme.bg_secondary))
                            // * The editor's own window listener runs ahead
                            //   of this element and would put the caret on
                            //   the fence line; Obsidian's flair leaves the
                            //   caret where it was, so the mouse stops here.
                            .block_mouse_except_scroll()
                            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                // * An empty fence copies nothing at all, the
                                //   same rule the reading view's button
                                //   follows -- and with nothing copied there
                                //   is nothing to announce.
                                if payload.is_empty() {
                                    return;
                                }
                                cx.write_to_clipboard(ClipboardItem::new_string(payload.clone()));
                                entity.update(cx, |_, cx| {
                                    cx.emit(EditorEvent::Reported(
                                        "Copied to your clipboard".into(),
                                    ))
                                });
                            })
                            .child(label)
                            .into_any_element();
                        let measured = element.layout_as_root(
                            size(AvailableSpace::MinContent, AvailableSpace::MinContent),
                            window,
                            cx,
                        );
                        // * Obsidian's rule is `right: 6px; top: 6px` against
                        //   a line box with padding of its own, which comes
                        //   out very nearly centred on the fence line: its
                        //   flair is 26 px tall on a 29 px line. Centring it
                        //   here is that picture without the padding.
                        element.prepaint_at(
                            point(
                                content_x + content_width - measured.width - px(6.),
                                y + (rh - measured.height).max(px(0.)) / 2.,
                            ),
                            window,
                            cx,
                        );
                        flair_elements.push(element);
                    }
                }
                // * The fold indicator: Obsidian's `cm-fold-indicator`, a
                //   hover-revealed chevron at `left: -15px` on a heading line
                //   and `-10px` on a list line, shown whenever the line is
                //   folded. Mouse-down rather than click, blocking the mouse,
                //   for the reason the callout chevron gives.
                if let Some(f) = foldables.iter().find(|f| f.line == line) {
                    let folded = editor.folds.is_folded(line);
                    if folded || hover_line == Some(line) {
                        let entity = entity.clone();
                        let offset = match f.kind {
                            fold::FoldKind::Heading(_) => px(15.),
                            fold::FoldKind::List => px(10.),
                        };
                        let glyph = if folded {
                            "chevron-right"
                        } else {
                            "chevron-down"
                        };
                        let mut element = div()
                            .id(("fold-indicator", line))
                            .size(px(16.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor(CursorStyle::PointingHand)
                            .block_mouse_except_scroll()
                            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                entity.update(cx, |e, cx| e.toggle_fold_at(line, cx));
                            })
                            .child(crate::assets::icon(glyph, 14., palette.muted))
                            .into_any_element();
                        element.layout_as_root(
                            size(AvailableSpace::MinContent, AvailableSpace::MinContent),
                            window,
                            cx,
                        );
                        element.prepaint_at(
                            point(
                                content_x - offset - px(8.),
                                y + (rh - px(16.)).max(px(0.)) / 2.,
                            ),
                            window,
                            cx,
                        );
                        flair_elements.push(element);
                    }
                }
                let range = editor.buf.line_range(line);
                if !matches.is_empty() {
                    // * Matches touching this line, by binary search on their starts.
                    let first = matches.partition_point(|m| m.end < range.start);
                    for (ix, m) in matches.iter().enumerate().skip(first) {
                        if m.start > range.end {
                            break;
                        }
                        let s = m.start.max(range.start) - range.start;
                        let e = m.end.min(range.end) - range.start;
                        let (rs, xs) = shaped.row_and_x(s);
                        let (re, xe) = shaped.row_and_x(e);
                        let color = if current_match == Some(ix) {
                            palette.find_current
                        } else {
                            palette.find_match
                        };
                        for r in rs..=re {
                            let x0 = if r == rs { xs } else { px(0.) };
                            let x1 = if r == re { xe } else { content_width };
                            if x1 > x0 {
                                decor_quads.push(fill(
                                    Bounds::new(
                                        point(content_x + x0, y + rh * r as f32),
                                        size(x1 - x0, rh),
                                    ),
                                    color,
                                ));
                            }
                        }
                    }
                }
                for sel in &cursors.sels {
                    let selection = sel.range();
                    if selection.start != selection.end
                        && selection.start <= range.end
                        && selection.end > range.start
                    {
                        let s = selection.start.max(range.start) - range.start;
                        let e = selection.end.min(range.end) - range.start;
                        let (rs, xs) = shaped.row_and_x(s);
                        let (re, xe) = shaped.row_and_x(e);
                        for r in rs..=re {
                            let x0 = if r == rs { xs } else { px(0.) };
                            let mut x1 = if r == re { xe } else { content_width };
                            if r == re && selection.end > range.end {
                                x1 += font_size * 0.4;
                            }
                            if x1 > x0 {
                                let sel_x = if editor.right_to_left {
                                    content_x + content_width - x1
                                } else {
                                    content_x + x0
                                };
                                selection_quads.push(fill(
                                    Bounds::new(point(sel_x, y + rh * r as f32), size(x1 - x0, rh)),
                                    palette.selection,
                                ));
                            }
                        }
                    }
                    if sel.head >= range.start && sel.head <= range.end {
                        let (row, x) = shaped.row_and_x(sel.head - range.start);
                        let cursor_x = if editor.right_to_left {
                            content_x + content_width - x - px(2.)
                        } else {
                            content_x + x
                        };
                        cursor_quads.push(fill(
                            Bounds::new(point(cursor_x, y + rh * row as f32), size(px(2.), rh)),
                            palette.text,
                        ));
                        if sel.head == head {
                            cursor_top = y;
                        }
                    }
                }
                lines.push((
                    VisibleLine {
                        line,
                        top: y - bounds.origin.y,
                        rows,
                        row_height: rh,
                    },
                    shaped,
                ));
                y += height;
                line += 1;
            }
            for w in &widgets {
                let height = w.bounds.size.height;
                let previous = editor.widget_heights.insert(w.lines.start, height);
                // * Whatever the reveal that got us here was computed from
                //   has to be what the measurement is checked against, or the
                //   first jump over a widget never gets its correction: a
                //   widget nothing had measured yet was estimated by
                //   `height_of` at one line each, and a table or a code fence
                //   is nothing like that tall. A widget that grew later -- an
                //   image that has just loaded -- is the same case with a real
                //   previous height.
                let estimate = previous.unwrap_or(editor.layout.line_height * w.lines.len() as f32);
                if estimate != height {
                    editor.pending_reveal = true;
                }
            }
            let cursor_line_ix = editor.cursor().line;
            let cursor_line = lines
                .iter()
                .any(|(v, _)| v.line == cursor_line_ix)
                .then(|| {
                    // * The input handler needs the cursor line's layout between
                    //   frames; shaping it again hits GPUI's per-frame layout cache.
                    Arc::new(shape_display(
                        window,
                        &text_style,
                        font_size,
                        lh,
                        wrap_width,
                        editor.display_for(cursor_line_ix),
                        highlights
                            .get(cursor_line_ix)
                            .map(Vec::as_slice)
                            .unwrap_or(&[]),
                        &palette,
                    ))
                });
            if !focused {
                cursor_quads.clear();
            }
            let fits = scroll_line == 0
                && scroll_px == px(0.)
                && line >= count
                && y <= bounds.bottom_left().y;
            let track = Bounds::new(
                point(
                    bounds.bottom_right().x - px(SCROLLBAR_WIDTH),
                    bounds.origin.y,
                ),
                size(px(SCROLLBAR_WIDTH), bounds.size.height),
            );
            // * The part of the first line scrolled off the top moves the
            //   thumb between lines, so it does not step.
            let first = lines
                .first()
                .filter(|(v, _)| v.line == scroll_line)
                .map(|(v, _)| v.row_height * v.rows as f32)
                .or_else(|| {
                    widgets
                        .first()
                        .filter(|w| w.visible.line == scroll_line)
                        .map(|w| w.bounds.size.height)
                })
                .filter(|height| *height > px(0.))
                .map_or(scroll_line as f32, |height| {
                    scroll_line as f32 + (scroll_px / height).clamp(0., 1.)
                });
            let last_top = editor.last_top_line(window);
            let scrollbar = Scrollbar::layout(track, count, first, last_top, fits);
            // * Indent guides: one vertical segment per tab level under each
            //   indented visible line, at content_x + tab_size * level.
            let indent_guides = if editor.show_indent_guide {
                let tab_size_px = font_size * editor.tab_size as f32;
                let mut guides = Vec::new();
                for (visible, _) in &lines {
                    let text = editor.buf.line_text(visible.line);
                    let mut tabs = 0usize;
                    for ch in text.chars() {
                        if ch == '\t' {
                            tabs += 1;
                        } else {
                            break;
                        }
                    }
                    for level in 0..tabs {
                        let x = content_x + tab_size_px * (level + 1) as f32;
                        guides.push((x, visible.top, visible.height()));
                    }
                }
                guides
            } else {
                Vec::new()
            };

            // * Spellcheck: find misspelled words on visible lines and
            //   compute their paint rectangles for squiggly underlines.
            let spellcheck_words = if editor.spellcheck {
                let mut words = Vec::new();
                for (visible, shaped) in &lines {
                    let text = editor.buf.line_text(visible.line);
                    for (start, end) in editor.spellchecker.misspelled(text) {
                        let unwrapped = &shaped.line.unwrapped_layout;
                        let start_x = unwrapped.x_for_index(start);
                        let end_x = unwrapped.x_for_index(end);
                        words.push((
                            start_x,
                            visible.top + visible.row_height - px(2.),
                            end_x - start_x,
                        ));
                    }
                }
                words
            } else {
                Vec::new()
            };

            Prepaint {
                lines,
                hitbox,
                widgets,
                flairs: flair_elements,
                decor: decor_quads,
                line_height: lh,
                font_size,
                text_style,
                wrap_width,
                content_x,
                line_number_gutter: gutter,
                indent_guides,
                spellcheck_words,
                selection: selection_quads,
                cursors: cursor_quads,
                cursor_line,
                cursor_top,
                scroll_px,
                scrollbar,
                thumb_dragging,
                prepaint_ms: started.elapsed().as_secs_f64() * 1000.0,
            }
        })
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        // * Vim mode: when vim_mode is on and not in insert mode, keystrokes
        //   are vim commands, not text input. Intercept them here so they never
        //   reach the buffer's default input handler. This must run during
        //   paint, not prepaint: `on_key_event` asserts it is called from a
        //   paint pass.
        let editor_entity = self.editor.clone();
        window.on_key_event(move |event: &KeyDownEvent, phase, _window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let editor = editor_entity.read(cx);
            let vim_on = editor.vim_mode && !editor.vim_insert_mode;
            let _ = editor;
            if !vim_on {
                return;
            }
            let key = event.keystroke.key.clone();
            match key.as_str() {
                "i" => {
                    cx.stop_propagation();
                    editor_entity.update(cx, |e, cx| {
                        e.vim_insert_mode = true;
                        cx.notify();
                    });
                }
                "Escape" => {
                    cx.stop_propagation();
                    editor_entity.update(cx, |e, cx| {
                        e.vim_insert_mode = false;
                        cx.notify();
                    });
                }
                "h" => {
                    cx.stop_propagation();
                    editor_entity.update(cx, |e, cx| {
                        if e.buf.head > 0 {
                            e.buf.head -= 1;
                        }
                        e.buf.anchor = e.buf.head;
                        e.buf.mark_dirty();
                        cx.notify();
                    });
                }
                "l" => {
                    cx.stop_propagation();
                    editor_entity.update(cx, |e, cx| {
                        e.buf.head = e.buf.head.saturating_add(1).min(e.buf.text().len());
                        e.buf.anchor = e.buf.head;
                        e.buf.mark_dirty();
                        cx.notify();
                    });
                }
                "j" => {
                    cx.stop_propagation();
                    editor_entity.update(cx, |e, cx| {
                        let starts = e.buf.line_starts();
                        let head = e.buf.head;
                        let mut cur = 0usize;
                        for (i, &s) in starts.iter().enumerate() {
                            if s <= head {
                                cur = i;
                            } else {
                                break;
                            }
                        }
                        if cur + 1 < starts.len() {
                            let line_start = starts[cur + 1];
                            let line_end = if cur + 2 < starts.len() {
                                starts[cur + 2]
                            } else {
                                e.buf.text().len()
                            };
                            let col = head - starts[cur];
                            let new_head = line_start + e.buf.clamp_col(cur + 1, col);
                            e.buf.head = new_head.min(line_end);
                            e.buf.anchor = e.buf.head;
                        }
                        e.buf.mark_dirty();
                        cx.notify();
                    });
                }
                "k" => {
                    cx.stop_propagation();
                    editor_entity.update(cx, |e, cx| {
                        let starts = e.buf.line_starts();
                        let head = e.buf.head;
                        let mut cur = 0usize;
                        for (i, &s) in starts.iter().enumerate() {
                            if s <= head {
                                cur = i;
                            } else {
                                break;
                            }
                        }
                        if cur > 0 {
                            let line_start = starts[cur - 1];
                            let line_end = starts[cur];
                            let col = head - starts[cur];
                            let new_head = line_start + e.buf.clamp_col(cur - 1, col);
                            e.buf.head = new_head.min(line_end);
                            e.buf.anchor = e.buf.head;
                        }
                        e.buf.mark_dirty();
                        cx.notify();
                    });
                }
                "x" => {
                    cx.stop_propagation();
                    editor_entity.update(cx, |e, cx| {
                        e.buf.delete_forward();
                        e.buf.mark_dirty();
                        cx.notify();
                    });
                }
                _ => {
                    cx.stop_propagation();
                }
            }
        });
        let focus_handle = self.editor.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.editor.clone()),
            cx,
        );
        let line_height = prepaint.line_height;
        let content_x = prepaint.content_x;
        let font_size = prepaint.font_size;
        let text_color = self.editor.read(cx).palette.text;
        let right_to_left = self.editor.read(cx).right_to_left;
        // * Cloned once: a borrowed palette would hold `cx` across every
        //   paint call in the loop below, and each of those needs it.
        let palette = self.editor.read(cx).palette.clone();
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            for quad in prepaint.decor.drain(..) {
                window.paint_quad(quad);
            }
            for quad in prepaint.selection.drain(..) {
                window.paint_quad(quad);
            }
            // * Indent guides: faint vertical lines under indented lines,
            //   painted behind the text.
            let mut guide = palette.muted;
            guide.a = 0.25;
            for (x, top, height) in &prepaint.indent_guides {
                window.paint_quad(fill(
                    Bounds::new(point(*x, bounds.origin.y + *top), size(px(1.), *height)),
                    guide,
                ));
            }
            // * Spellcheck squiggly underlines: draw a sine-wave path under
            //   each misspelled word.
            let mut misspelled = palette.text;
            misspelled.a = 1.0;
            for (x, top, width) in &prepaint.spellcheck_words {
                let mut path_builder = PathBuilder::fill();
                let baseline = bounds.origin.y + *top + px(6.);
                let steps = 8;
                let step_w = *width / steps as f32;
                path_builder.move_to(point(*x, baseline));
                for s in 1..=steps {
                    let t = s as f32 * std::f32::consts::PI * 2.0 / steps as f32;
                    let y = baseline + t.sin() * px(2.);
                    path_builder.line_to(point(*x + step_w * s as f32, y));
                }
                let path = path_builder.build().unwrap();
                window.paint_path(path, misspelled);
            }
            for (visible, shaped) in prepaint.lines.iter() {
                let origin = point(content_x, bounds.origin.y + visible.top);
                // * Line numbers in the gutter, right-aligned.
                if prepaint.line_number_gutter > px(0.) {
                    let theme = cx.global::<Theme>();
                    let number = (visible.line + 1).to_string();
                    let mut run = prepaint.text_style.to_run(number.len());
                    run.color = theme.text_faint;
                    let line_no = window.text_system().shape_line(
                        SharedString::from(number),
                        font_size,
                        &[run],
                        None,
                    );
                    let origin = point(
                        content_x - prepaint.line_number_gutter + px(4.),
                        bounds.origin.y + visible.top,
                    );
                    let _ = line_no.paint(
                        origin,
                        visible.row_height,
                        TextAlign::Right,
                        None,
                        window,
                        cx,
                    );
                }
                // * Run backgrounds (code, highlights) are a separate pass.
                let _ = shaped.line.paint_background(
                    origin,
                    visible.row_height,
                    if right_to_left {
                        TextAlign::Right
                    } else {
                        TextAlign::Left
                    },
                    None,
                    window,
                    cx,
                );
                let _ = shaped.line.paint(
                    origin,
                    visible.row_height,
                    if right_to_left {
                        TextAlign::Right
                    } else {
                        TextAlign::Left
                    },
                    None,
                    window,
                    cx,
                );
                // * Inline formulas over their placeholders, on the text
                //   baseline and centred in the placeholder's slack, in the
                //   text colour: rendered math is not markup.
                for m in &shaped.display.math {
                    let Ok(formula) = silica_math::typeset(&m.tex, m.block) else {
                        continue;
                    };
                    let em = font_size * shaped.display.scale;
                    let rh = visible.row_height;
                    let layout = &shaped.line.unwrapped_layout;
                    let Some(start) = shaped.line.position_for_index(m.display.start, rh) else {
                        continue;
                    };
                    let width = crate::math::width_px(&formula, em);
                    let end_x = shaped
                        .line
                        .position_for_index(m.display.end, rh)
                        .filter(|end| end.y == start.y)
                        .map(|end| end.x)
                        .unwrap_or(start.x + width);
                    let slack = (end_x - start.x - width).max(px(0.));
                    let baseline = point(
                        origin.x + start.x + slack / 2.,
                        origin.y
                            + start.y
                            + (rh - layout.ascent - layout.descent) / 2.
                            + layout.ascent,
                    );
                    crate::math::paint(&formula, baseline, em, text_color, window);
                }
                // * Task boxes over their placeholders, from the same
                //   `--checkbox-*` tokens the reading view draws them with.
                paint_task_boxes(
                    shaped,
                    origin,
                    font_size * shaped.display.scale,
                    &palette,
                    window,
                );
            }
            for w in prepaint.widgets.iter_mut() {
                w.element.paint(window, cx);
            }
            // * Over the lines: a flair sits on its fence line's text.
            for flair in prepaint.flairs.iter_mut() {
                flair.paint(window, cx);
            }
            for cursor in prepaint.cursors.drain(..) {
                window.paint_quad(cursor);
            }
            if let Some(bar) = prepaint.scrollbar {
                let mut color = self.editor.read(cx).palette.muted;
                color.a = if prepaint.thumb_dragging { 0.6 } else { 0.35 };
                window.paint_quad(fill(bar.thumb, color).corner_radii(px(3.)));
            }
        });

        let editor = self.editor.clone();
        let hitbox = prepaint.hitbox.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble
                && event.button == MouseButton::Left
                && hitbox.is_hovered(window)
            {
                editor.update(cx, |editor, cx| editor.mouse_down(event, window, cx));
            }
            if phase == DispatchPhase::Bubble
                && event.button == MouseButton::Right
                && hitbox.is_hovered(window)
            {
                let at = editor.read(cx).offset_at_point(window, event.position);
                editor.update(cx, |editor, cx| {
                    editor.context_menu = Some(ContextMenu {
                        position: event.position,
                        target: at,
                    });
                    // * Place the caret at the click unless it lands inside
                    //   the current selection, which stays intact.
                    if let Some(at) = at {
                        let sels = editor.buf.cursors();
                        let inside = sels
                            .sels
                            .iter()
                            .any(|s| !s.is_empty() && s.head >= at && s.head <= at);
                        if !inside {
                            editor.buf.head = at;
                            editor.buf.anchor = at;
                        }
                    }
                    cx.notify();
                });
            }
        });
        let editor = self.editor.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            if event.pressed_button == Some(MouseButton::Left) {
                editor.update(cx, |editor, cx| {
                    editor.mouse_drag(event.position, window, cx)
                });
            } else if event.pressed_button.is_none() {
                editor.update(cx, |editor, cx| {
                    editor.hover_at(event.position, event.modifiers, window, cx)
                });
            }
        });
        let editor = self.editor.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _window, cx| {
            if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                editor.update(cx, |editor, cx| {
                    editor.dragging = false;
                    if editor.thumb_drag.take().is_some() {
                        cx.notify();
                    }
                });
            }
        });
        let editor = self.editor.clone();
        let hitbox = prepaint.hitbox.clone();
        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
            // * `should_handle_scroll` rather than `is_hovered`: a wheel event
            //   is not an interaction with what is directly under the pointer.
            if phase == DispatchPhase::Bubble && hitbox.should_handle_scroll(window) {
                let delta = event.delta.pixel_delta(line_height).y;
                editor.update(cx, |editor, cx| editor.scroll_by(-delta, window, cx));
            }
        });

        let mut visible: Vec<VisibleLine> = prepaint
            .lines
            .iter()
            .map(|(v, _)| *v)
            .chain(prepaint.widgets.iter().map(|w| w.visible))
            .collect();
        visible.sort_by_key(|v| v.line);
        let layout = Layout {
            bounds,
            widgets: prepaint
                .widgets
                .iter()
                .map(|w| WidgetBox {
                    lines: w.lines.clone(),
                    bounds: w.bounds,
                    embed: w.embed,
                })
                .collect(),
            line_height,
            font_size: prepaint.font_size,
            text_style: Some(prepaint.text_style.clone()),
            wrap_width: prepaint.wrap_width,
            content_x,
            visible,
            cursor_line: prepaint.cursor_line.take(),
            cursor_top: prepaint.cursor_top,
            scrollbar: prepaint.scrollbar,
            prepaint_ms: prepaint.prepaint_ms,
        };
        let scroll_px = prepaint.scroll_px;
        self.editor.update(cx, |editor, cx| {
            editor.frame_samples.add(layout.prepaint_ms);
            editor.layout = layout;
            editor.scroll_px = scroll_px;
            if editor.pending_reveal {
                editor.pending_reveal = false;
                editor.reveal_cursor(window);
                // * A notify during paint is dropped; ask for the next frame.
                let id = cx.entity_id();
                window.on_next_frame(move |_, cx| cx.notify(id));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The display line for the first line of `src`, nothing touched.
    fn display(src: &str) -> DisplayLine {
        let doc = silica_markdown::parse(src);
        let lines = highlight_lines(&doc);
        preview::display_line(src.lines().next().unwrap_or(""), &lines[0], &[])
    }

    /// Every flair in `src`, as `(line, label, copied)`.
    fn flairs(src: &str) -> Vec<(usize, String, String)> {
        let doc = silica_markdown::parse(src);
        code_flairs(src, &doc)
            .into_iter()
            .map(|f| (f.line, f.label.to_string(), f.copy))
            .collect()
    }

    #[test]
    fn a_flair_per_fence_with_obsidians_label() {
        // * The six display names that were read out of a running Obsidian,
        //   an unregistered info string kept verbatim, and a fence with none
        //   at all, whose flair is there with an empty label.
        let src = "```js\nconst CASE_LANG = 1;\n```\n\n```silica-made-up\nunregistered info\n```\n\n```\nno info string\n```\n";
        assert_eq!(
            flairs(src),
            vec![
                (0, "JavaScript".into(), "const CASE_LANG = 1;".into()),
                (4, "silica-made-up".into(), "unregistered info".into()),
                (8, String::new(), "no info string".into()),
            ]
        );
        for (info, want) in [
            ("ts", "TypeScript"),
            ("python", "Python"),
            ("rust", "Rust"),
            ("c", "C"),
            ("yaml", "YAML"),
            ("YAML", "YAML"),
        ] {
            assert_eq!(language_display_name(info), want, "{info}");
        }
    }

    #[test]
    fn an_indented_block_has_no_flair_and_a_nested_fence_has_one() {
        // * A note of eight blocks whose only indented one was the fifth
        //   showed seven flairs, so the fence line is what carries it.
        assert_eq!(flairs("    four space indented\n    second line\n"), vec![]);
        assert_eq!(
            flairs("> ```\n> in a blockquote\n> ```\n"),
            vec![(0, String::new(), "in a blockquote".into())]
        );
        assert_eq!(
            flairs("- item\n\n  ```js\n  in a list\n  ```\n"),
            vec![(2, "JavaScript".into(), "in a list".into())]
        );
        // * A CRLF file's block copies as LF, and exactly one trailing
        //   newline comes off -- the rule the reading view's button follows.
        assert_eq!(
            flairs("```\r\none\r\ntwo\r\n\r\n```\r\n"),
            vec![(0, String::new(), "one\ntwo\n".into())]
        );
        // * An empty fence carries a flair that copies nothing; the click
        //   path writes nothing rather than emptying the clipboard.
        assert_eq!(
            flairs("```\n```\n"),
            vec![(0, String::new(), String::new())]
        );
    }

    #[test]
    fn end_of_display_is_the_end_of_the_source_line() {
        // * Trailing markup displays nothing, so `to_source` has no piece to
        //   match at the end and walks back into the link text; the click
        //   path must land after the `]]` instead.
        let d = display("see [[Welcome]]");
        assert_eq!(d.text, "see Welcome");
        assert_eq!(d.to_source(d.text.len()), 13);
        assert_eq!(source_index(&d, d.text.len()), 15);

        let d = display("ab `cd`");
        assert_eq!(d.text, "ab cd");
        assert_eq!(source_index(&d, d.text.len()), 7);

        let d = display("- [ ] task");
        assert_eq!(d.text, format!("{} task", crate::preview::TASK_BOX));
        assert_eq!(source_index(&d, d.text.len()), 10);
    }

    /// A done task's strikethrough stops at its box: the gap between the box
    /// and the words is not struck, which would read as a stray `-`.
    #[test]
    fn a_done_task_leaves_the_gap_after_its_box_unstruck() {
        let d = display("- [x] done");
        let runs = runs_for_display(&d, &[], &gpui::TextStyle::default(), &Palette::default());
        let gap = d.tasks[0].display.end;
        let mut at = 0;
        let mut struck = Vec::new();
        for run in &runs {
            struck.push((at..at + run.len, run.strikethrough.is_some()));
            at += run.len;
        }
        assert_eq!(at, d.text.len(), "the runs cover the text");
        for (range, strike) in struck {
            if range.start > gap {
                assert!(strike, "the words are struck: {range:?}");
            } else if range.start >= gap {
                assert!(!strike, "the space after the box is not: {range:?}");
            }
        }
    }

    /// The control socket's `type` writes through the same insertion a
    /// keystroke takes, so a string typed one character at a time must
    /// leave what pressing those keys leaves: `*` over a selection wraps
    /// the word rather than replacing it, and the `]]` the editor itself
    /// closed is stepped over rather than doubled.
    #[test]
    fn a_typed_character_pairs_whether_it_arrives_as_a_key_or_in_a_string() {
        let opts = autopair::Options::default();
        let mut buf = TextBuffer::new();
        buf.load("alpha beta gamma");
        buf.anchor = 0;
        buf.head = 5;
        assert!(Editor::type_char_in(&mut buf, "*", EditKind::Typing, opts));
        assert_eq!(buf.text(), "*alpha* beta gamma");
        assert_eq!((buf.anchor, buf.head), (1, 6));

        let mut buf = TextBuffer::new();
        let mut utf8 = [0u8; 4];
        for ch in "[[Welcome]]".chars() {
            Editor::type_char_in(&mut buf, ch.encode_utf8(&mut utf8), EditKind::Typing, opts);
        }
        assert_eq!(buf.text(), "[[Welcome]]");
        assert_eq!(buf.head, 11);

        // * A string that is not one character is not a keystroke: it goes
        //   in as it stands, the way a paste does.
        let mut buf = TextBuffer::new();
        assert!(Editor::type_char_in(&mut buf, "((", EditKind::Typing, opts));
        assert_eq!(buf.text(), "((");
    }

    #[test]
    fn inside_the_display_still_maps_through_the_pieces() {
        let d = display("see [[Welcome]]");
        for i in 0..d.text.len() {
            assert_eq!(source_index(&d, i), d.to_source(i), "display byte {i}");
        }
        // * The last glyph of the link text is still inside the link's span,
        //   which is what makes a click on it follow the link.
        assert!(source_index(&d, 10) < 15);

        let d = display("plain line");
        assert_eq!(source_index(&d, 4), 4);
        assert_eq!(source_index(&d, d.text.len()), 10);
    }
}
