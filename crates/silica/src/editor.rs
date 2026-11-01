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
use crate::math::SpaceWidths;
use crate::preview::{self, DisplayLine, Piece};
use crate::properties::{self, Field, Part, PillStep, Target};
use crate::reading::model::{build_widgets, BuildOptions, EmbedSource, LinkTarget, RBlock, Widget};
use crate::reading::view::{LinkHandler, Renderer, ToggleHandler};
use crate::reading::view::{PropertiesHost, PropertyAction};
use crate::reading::view::{TableAction, TableHost};
use crate::suggest::{Anchor, SuggestState};
use crate::text_input::{InputEvent, TextInput};
use crate::theme::Theme;
use crate::workspace::VaultState;
use gpui::{
    actions, div, fill, point, prelude::*, px, relative, size, AnyElement, App, AvailableSpace,
    Bounds, ClipboardItem, Context, CursorStyle, DispatchPhase, Element, ElementId,
    ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable, Font,
    FontStyle, FontWeight, GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId,
    KeyBinding, LayoutId, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad,
    Pixels, Point, ScrollWheelEvent, SharedString, StrikethroughStyle, Style, Subscription, Task,
    TextAlign, TextRun, TextStyle, UTF16Selection, UnderlineStyle, Window, WrappedLine,
};
use silica_formats::config::PropertyTypes;
use silica_formats::properties::{self as props, PropertiesInDocument};
use silica_markdown::highlight::{flatten, highlight_lines, HighlightKind, HighlightSpan};
use silica_markdown::{Block, BlockKind, Document};
use silica_suggest::blocks::{self, IdInsertion, Placement};
use silica_suggest::{self as suggest, Accept, BlockId, Suggestion};
use silica_text::table::{self, TableCommand};
use silica_text::{
    autopair, find, shifted, wrap, BlockInsert, Cursor, Cursors, EditKind, LinePrefix, LinkSyntax,
    Sel, TextBuffer,
};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

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
        // The popup's own keys, in the context the editor only carries
        // while it is open, so Ctrl+N still makes a new note otherwise.
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
struct Shaped {
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
            // gpui reports a byte that falls on a wrap point at the right
            // edge of the row before, which strands the caret a row above
            // the text it is in and leaves Home with nowhere for End to go.
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
struct Hit {
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
    readable_line_length: bool,
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
            // A view that has not been given a file yet belongs to none,
            // and a placeholder name here would read as one: the shell
            // would mount this editor on launch over an empty vault and
            // take a whole note's worth of typing that nothing saves.
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
            readable_line_length: true,
            live_preview: false,
            smart_indent_list: true,
            use_tab: true,
            tab_size: 4,
            frame_samples: Sample::default(),
            parse_samples: Sample::default(),
            parse_latency: Sample::default(),
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
        // Fold ids are per-note counters, so another note's would collapse
        // whichever callout happened to land on the same number.
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
        // The matches point into the note that was open; drop them rather
        // than let the panel count or step through another note's offsets.
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
        self.after_edit(Some(window), cx);
        true
    }

    /// `editor:insert-table`: an empty two-by-two table after the cursor's
    /// line, with the cursor in its first header cell.
    pub fn insert_table(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.buf.insert_table() {
            self.after_edit(Some(window), cx);
        }
    }

    /// One of Obsidian's formatting toggles: wraps each selection in
    /// `marker`, or takes the marker off when it is already there.
    pub fn toggle_wrap(&mut self, marker: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.buf.toggle_wrap(marker) {
            self.after_edit(Some(window), cx);
        }
    }

    /// `editor:toggle-checklist-status` (Mod+L): a checkbox on every
    /// selected line that has none, or the one it has flipped, writing
    /// the same bytes a click on the rendered checkbox writes.
    pub fn toggle_checklist(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.buf.toggle_checklist() {
            self.after_edit(Some(window), cx);
        }
    }

    /// `editor:cycle-list-checklist`: bullet, `[ ]`, `[x]`, bullet.
    pub fn cycle_checklist(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.buf.cycle_checklist() {
            self.after_edit(Some(window), cx);
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
            self.after_edit(Some(window), cx);
        }
    }

    /// `editor:clear-formatting`: the markers inside the selection go.
    pub fn clear_formatting(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.buf.clear_formatting() {
            self.after_edit(Some(window), cx);
        }
    }

    /// One of the six insert-block commands at the caret.
    pub fn insert_block(&mut self, kind: BlockInsert, window: &mut Window, cx: &mut Context<Self>) {
        if self.buf.insert_block(kind) {
            self.after_edit(Some(window), cx);
        }
    }

    /// `editor:set-heading-<n>`: rewrites the selected lines' `#`s.
    pub fn set_heading(&mut self, level: u8, window: &mut Window, cx: &mut Context<Self>) {
        if self.buf.set_heading(level) {
            self.after_edit(Some(window), cx);
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
            self.after_edit(Some(window), cx);
        }
    }

    /// `editor:delete-paragraph`.
    pub fn delete_paragraph(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.buf.delete_paragraph() {
            self.after_edit(Some(window), cx);
        }
    }

    /// `editor:swap-line-up` and `-down`.
    pub fn swap_lines(&mut self, up: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.buf.swap_lines(up) {
            self.after_edit(Some(window), cx);
        }
    }

    /// `editor:insert-link`, `-wikilink` and `-embed`: the link shape
    /// around the selection, with the caret where the target is typed —
    /// which is where `after_edit` opens the autocompletion popup.
    pub fn insert_link(&mut self, kind: LinkSyntax, window: &mut Window, cx: &mut Context<Self>) {
        if self.buf.insert_link(kind) {
            self.after_edit(Some(window), cx);
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
            self.after_edit(Some(window), cx);
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
        // Not `rendered_only`: a cursor inside a link is exactly what
        // makes Live Preview reveal its source.
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
            self.after_edit(Some(window), cx);
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
        // Like any other cursor move: refresh a popup that is open, and
        // close it once the cursor has left its query.
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
            // An inline formula is shaped as a placeholder the width it
            // typesets to, at the body size: a heading's scale is applied
            // to the shaped line, placeholder included.
            let math = |tex: &str, block: bool| -> Option<String> {
                let (size, spaces) = self.spaces.as_ref()?;
                let formula = silica_math::typeset(tex, block).ok()?;
                Some(spaces.placeholder(crate::math::width_px(&formula, *size)).0)
            };
            preview::display_line_with(text, spans, &self.touches_on_line(line), &math)
        } else {
            DisplayLine::verbatim(text, 1.0, Default::default())
        };
        // A folded line shows `…` after its text in both editing modes:
        // raw source mode was observed folding exactly like Live Preview.
        if self.folds.is_folded(line) {
            return display.with_fold_placeholder(text.len());
        }
        display
    }

    // ----- folding --------------------------------------------------------------

    /// `foldHeading` and `foldIndent`, read from the vault's `app.json`
    /// as each parse starts; both default to on.
    fn fold_options(&self) -> fold::FoldOptions {
        let Some(state) = &self.suggest_vault else {
            return fold::FoldOptions::default();
        };
        let app = state.app();
        fold::FoldOptions {
            headings: app.fold_heading.unwrap_or(true),
            lists: app.fold_indent.unwrap_or(true),
        }
    }

    /// Settles the folds against the current foldables and recomputes the
    /// hidden lines.
    fn refresh_folds(&mut self) {
        self.hidden = self.folds.resolve(&self.foldables);
    }

    /// A cursor that arrived inside a fold — a link into the section, a
    /// search hit, an edit — opens what hides it, the way a link into a
    /// folded section was observed expanding it. A fold command that closes
    /// around the caret leaves the caret where it is, as CodeMirror's does:
    /// fold-less from inside the folded section still finds that section.
    fn unfold_at_cursor(&mut self) {
        let line = self.buf.cursor_at(self.buf.head).line;
        if fold::is_hidden(&self.hidden, line) && self.folds.unfold_hiding(&self.foldables, line) {
            self.refresh_folds();
        }
    }

    fn line_hidden(&self, line: usize) -> bool {
        fold::is_hidden(&self.hidden, line)
    }

    /// The foldable starting on `line`, if one does.
    fn foldable_at(&self, line: usize) -> Option<&Foldable> {
        self.foldables.iter().find(|f| f.line == line)
    }

    /// `editor:toggle-fold`: folds or unfolds the caret's line. Obsidian's
    /// command reports success on a line that starts no fold and changes
    /// nothing, which is what this does too.
    pub fn toggle_fold(&mut self, cx: &mut Context<Self>) {
        let line = self.buf.cursor_at(self.buf.head).line;
        self.toggle_fold_at(line, cx);
    }

    /// Folds or unfolds the foldable starting on `line`: the command on
    /// the caret's line, or a click on a line's indicator.
    fn toggle_fold_at(&mut self, line: usize, cx: &mut Context<Self>) {
        if self.folds.toggle(&self.foldables, line) {
            self.refresh_folds();
            cx.notify();
        }
    }

    /// `editor:fold-all`.
    pub fn fold_all(&mut self, cx: &mut Context<Self>) {
        self.folds.fold_all(&self.foldables);
        self.refresh_folds();
        cx.notify();
    }

    /// `editor:unfold-all`.
    pub fn unfold_all(&mut self, cx: &mut Context<Self>) {
        self.folds.unfold_all();
        self.refresh_folds();
        cx.notify();
    }

    /// `editor:fold-more` and `editor:fold-less`: one enclosing level out
    /// from the caret's section, or the outermost one back.
    pub fn fold_step(&mut self, more: bool, cx: &mut Context<Self>) {
        let line = self.buf.cursor_at(self.buf.head).line;
        let changed = if more {
            self.folds.fold_more(&self.foldables, line)
        } else {
            self.folds.fold_less(&self.foldables, line)
        };
        if changed {
            self.refresh_folds();
            cx.notify();
        }
    }

    /// The folds by key, for the reading view to take over on a mode
    /// switch.
    pub fn fold_keys(&self) -> Vec<fold::FoldKey> {
        self.folds.keys()
    }

    /// The reading view's folds, on a switch back to editing.
    pub fn set_fold_keys(&mut self, keys: Vec<fold::FoldKey>, cx: &mut Context<Self>) {
        self.folds = fold::Folds::from_keys(keys);
        self.pending_folds = None;
        self.refresh_folds();
        cx.notify();
    }

    /// The note's fold record for the store: the full extents, as the
    /// editor keeps them, plus `{0,0}` for a folded properties block.
    /// `None` while the first parse has not delivered the foldables a
    /// restored record is resolved against, so a note left at once keeps
    /// the record it was opened with.
    pub fn fold_record(&self) -> Option<fold::Record> {
        if self.pending_folds.is_some() {
            return None;
        }
        let mut folds: Vec<fold::FoldRange> = self
            .folds
            .record(&self.foldables)
            .into_iter()
            .map(|(from, to)| fold::FoldRange { from, to })
            .collect();
        if self.properties_folded {
            folds.insert(0, fold::PROPERTIES_RANGE);
        }
        Some(fold::Record {
            folds,
            lines: self.buf.line_count(),
        })
    }

    /// Restores a note's record: each `from` line that starts a fold folds
    /// once the parse has said which lines do.
    pub fn set_fold_record(&mut self, record: &fold::Record, cx: &mut Context<Self>) {
        self.properties_folded = record.folds.contains(&fold::PROPERTIES_RANGE);
        let froms: Vec<usize> = record
            .folds
            .iter()
            .filter(|r| **r != fold::PROPERTIES_RANGE)
            .map(|r| r.from)
            .collect();
        // Before the first parse there is nothing to resolve against: the
        // `from` lines wait for it. (A note with nothing foldable keeps
        // them waiting, which costs nothing.)
        if self.foldables.is_empty() {
            self.pending_folds = Some(froms);
        } else {
            self.folds = fold::Folds::from_record(&self.foldables, froms);
            self.refresh_folds();
        }
        cx.notify();
    }

    pub fn properties_folded(&self) -> bool {
        self.properties_folded
    }

    pub fn set_properties_folded(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.properties_folded != on {
            self.properties_folded = on;
            cx.notify();
        }
    }

    /// The line under window point `position`, from the last frame.
    fn line_at_point(&self, position: Point<Pixels>) -> Option<usize> {
        let layout = &self.layout;
        if !layout.bounds.contains(&position) {
            return None;
        }
        let y = position.y - layout.bounds.origin.y;
        layout
            .visible
            .iter()
            .find(|v| y >= v.top && y < v.top + v.height())
            .map(|v| v.line)
    }

    /// Remembers the hovered line, which is where the fold indicator shows,
    /// and updates the hovered link preview when Ctrl/Cmd is held.
    fn hover_at(
        &mut self,
        position: Point<Pixels>,
        modifiers: Modifiers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let line = self.line_at_point(position);
        if line != self.hover_line {
            // Only a change that moves an indicator is worth a frame.
            let affects = |l: Option<usize>| l.is_some_and(|l| self.foldable_at(l).is_some());
            let repaint = affects(line) || affects(self.hover_line);
            self.hover_line = line;
            if repaint {
                cx.notify();
            }
        }

        if modifiers.control || modifiers.platform {
            if let Some(hit) = self.display_hit(window, position) {
                let col = self
                    .buf
                    .clamp_col(hit.line, hit.shaped.to_source(hit.display));
                if let Some(target) = self.link_at(hit.line, col, false) {
                    if let Some(state) = &self.suggest_vault {
                        let source = if self.name.is_empty() {
                            None
                        } else {
                            Some(self.name.as_ref())
                        };
                        let data = crate::page_preview::resolve_preview(&target, source, state);
                        let changed = match (&self.hover_preview, &data) {
                            (Some((_, old)), Some(new)) => old != new,
                            (Some(_), None) => true,
                            (None, Some(_)) => true,
                            (None, None) => false,
                        };
                        if changed {
                            self.hover_preview = data.map(|d| (position, d));
                            cx.notify();
                        }
                        return;
                    }
                }
            }
        }

        if self.hover_preview.is_some() {
            self.hover_preview = None;
            cx.notify();
        }
    }

    /// The widget covering `line` when Live Preview shows it rendered: no
    /// selection touches its lines, so its source is folded away.
    fn folded_widget(&self, line: usize) -> Option<&(Range<usize>, Widget)> {
        if !self.live_preview {
            return None;
        }
        let ix = self.widgets.partition_point(|(l, _)| l.end <= line);
        let entry = self.widgets.get(ix)?;
        if !entry.0.contains(&line) || entry.0.end > self.line_count() {
            return None;
        }
        self.widget_is_folded(entry).then_some(entry)
    }

    /// A widget that takes no room and draws nothing: the properties block
    /// under `propertiesInDocument: hidden`, where Obsidian shows neither
    /// the table nor the YAML behind it. Folding it to one line's height,
    /// which is what every other widget gets, leaves a band of empty space
    /// where the block used to be.
    fn widget_is_hidden(&self, widget: &Widget) -> bool {
        matches!(widget.block, RBlock::Properties(_))
            && self.properties_in_document == PropertiesInDocument::Hidden
    }

    fn widget_is_folded(&self, entry: &(Range<usize>, Widget)) -> bool {
        // The properties block stays a table wherever the cursor is: it is
        // edited in its own fields, not by revealing the YAML. `Show as
        // source` in `propertiesInDocument` is what reveals it instead.
        if matches!(entry.1.block, RBlock::Properties(_)) {
            return self.properties_in_document != PropertiesInDocument::Source;
        }
        // A table stays a grid wherever the cursor is, the way Obsidian's
        // `table-editor` does: its cells are typed into directly.
        if matches!(entry.1.block, RBlock::Table { .. }) {
            return true;
        }
        let span = self.buf.line_starts()[entry.0.start]..self.buf.line_range(entry.0.end - 1).end;
        let primary = Sel {
            anchor: self.buf.anchor,
            head: self.buf.head,
            goal: None,
        };
        !std::iter::once(&primary)
            .chain(self.buf.extra.iter())
            .any(|s| preview::touches(&[s.range()], &span))
    }

    /// Shapes `line` the way the last frame did (same style and wrap width).
    fn shape(&self, window: &mut Window, line: usize) -> Option<Shaped> {
        let style = self.layout.text_style.as_ref()?;
        let spans = self.highlights.get(line).map(Vec::as_slice).unwrap_or(&[]);
        Some(shape_display(
            window,
            style,
            self.layout.font_size,
            self.layout.line_height,
            self.layout.wrap_width,
            self.display_for(line),
            spans,
            &self.palette,
        ))
    }

    fn rows_of(&self, window: &mut Window, line: usize) -> usize {
        self.shape(window, line).map(|l| l.rows()).unwrap_or(1)
    }

    /// Height of `line` on screen: its wrapped rows at its row height. A
    /// folded widget's height sits on its first line; its other lines are
    /// not on screen at all.
    fn height_of(&self, window: &mut Window, line: usize) -> Pixels {
        if self.line_hidden(line) {
            return px(0.);
        }
        if let Some((lines, widget)) = self.folded_widget(line) {
            if line != lines.start || self.widget_is_hidden(widget) {
                return px(0.);
            }
            return self
                .widget_heights
                .get(&lines.start)
                .copied()
                .unwrap_or(self.layout.line_height * lines.len() as f32);
        }
        self.shape(window, line)
            .map(|l| l.height())
            .unwrap_or(self.layout.line_height)
    }

    fn visible_rows(&self) -> usize {
        let h = self.layout.bounds.size.height;
        let lh = self.layout.line_height;
        if lh > px(0.) {
            ((h / lh).floor() as usize).max(1)
        } else {
            40
        }
    }

    /// The last line the view can start at: scrolled there, the note's end
    /// sits at the bottom of the view (what `clamp_scroll_end` enforces).
    /// Zero when the whole note fits. Walks back from the end over one
    /// screenful of lines, so it costs what the visible lines do.
    fn last_top_line(&self, window: &mut Window) -> usize {
        let h = self.layout.bounds.size.height;
        if h <= px(0.) {
            return 0;
        }
        let mut filled = px(0.);
        let mut line = self.line_count();
        while line > 0 {
            line -= 1;
            filled += self.height_of(window, line);
            if filled >= h {
                return line;
            }
        }
        0
    }

    // ----- scrolling -----------------------------------------------------------

    /// Brings `scroll_px` back inside `scroll_line`'s rows.
    fn normalize_scroll(&mut self, window: &mut Window) {
        let lh = self.layout.line_height;
        if lh <= px(0.) {
            return;
        }
        while self.scroll_px < px(0.) {
            if self.scroll_line == 0 {
                self.scroll_px = px(0.);
                break;
            }
            self.scroll_line -= 1;
            self.scroll_px += self.height_of(window, self.scroll_line);
        }
        loop {
            let height = self.height_of(window, self.scroll_line);
            if self.scroll_px >= height && self.scroll_line + 1 < self.line_count() {
                self.scroll_px -= height;
                self.scroll_line += 1;
            } else {
                if self.scroll_px >= height {
                    self.scroll_px = (height - lh).max(px(0.));
                }
                break;
            }
        }
    }

    /// Keeps the last line from scrolling above the bottom of the view.
    fn clamp_scroll_end(&mut self, window: &mut Window) {
        let lh = self.layout.line_height;
        let h = self.layout.bounds.size.height;
        if lh <= px(0.) || h <= px(0.) {
            return;
        }
        let mut filled = -self.scroll_px;
        let mut line = self.scroll_line;
        while line < self.line_count() && filled < h {
            filled += self.height_of(window, line);
            line += 1;
        }
        if filled < h {
            self.scroll_px -= h - filled;
            self.normalize_scroll(window);
        }
    }

    /// Scrolls the view down by `dy` (up when negative).
    fn scroll_by(&mut self, dy: Pixels, window: &mut Window, cx: &mut Context<Self>) {
        if self.hover_preview.take().is_some() {
            cx.notify();
        }
        self.scroll_px += dy;
        self.normalize_scroll(window);
        if dy > px(0.) {
            self.clamp_scroll_end(window);
        }
        cx.notify();
    }

    /// Puts `line` at the top of the view (clamped so the note's end stays
    /// at the bottom), from the scrollbar and from the control socket.
    pub fn scroll_to_line(&mut self, line: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.scroll_line = line.min(self.line_count() - 1);
        self.scroll_px = px(0.);
        self.clamp_scroll_end(window);
        cx.notify();
    }

    /// Mouse down on the scrollbar: grabs the thumb, or jumps so the thumb
    /// centers on the click. Returns false when the point is off the track.
    fn scrollbar_mouse_down(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(bar) = self.layout.scrollbar else {
            return false;
        };
        if !bar.track.contains(&position) {
            return false;
        }
        if bar.thumb.contains(&position) {
            self.thumb_drag = Some(position.y - bar.thumb.origin.y);
        } else {
            let half = bar.thumb.size.height / 2.;
            self.thumb_drag = Some(half);
            let line = bar.line_for_thumb_top(position.y - half);
            self.scroll_to_line(line, window, cx);
        }
        true
    }

    fn scrollbar_drag(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(grab), Some(bar)) = (self.thumb_drag, self.layout.scrollbar) else {
            return;
        };
        let line = bar.line_for_thumb_top(position.y - grab);
        if line != self.scroll_line || self.scroll_px != px(0.) {
            self.scroll_to_line(line, window, cx);
        }
    }

    /// Scrolls the least amount that puts the cursor's row on screen.
    fn reveal_cursor(&mut self, window: &mut Window) {
        let lh = self.layout.line_height;
        let h = self.layout.bounds.size.height;
        if lh <= px(0.) || h <= px(0.) {
            self.pending_reveal = true;
            return;
        }
        let c = self.buf.cursor_at(self.buf.head);
        let Some(shaped) = self.shape(window, c.line) else {
            self.pending_reveal = true;
            return;
        };
        let (row, _) = shaped.row_and_x(c.col);
        let rh = shaped.row_height;
        let row_px = rh * row as f32;
        let center = std::mem::take(&mut self.pending_center);
        if center {
            self.scroll_row_to(c.line, row_px, (h - rh) / 2., window);
            return;
        }
        if c.line < self.scroll_line || (c.line == self.scroll_line && row_px < self.scroll_px) {
            self.scroll_line = c.line;
            self.scroll_px = row_px;
            return;
        }
        // Walk down from the anchor; stop once past the viewport.
        let mut y = -self.scroll_px;
        let mut line = self.scroll_line;
        let mut cursor_y = None;
        while line <= c.line && y <= h {
            if line == c.line {
                cursor_y = Some(y + row_px);
                break;
            }
            y += self.height_of(window, line);
            line += 1;
        }
        if let Some(cy) = cursor_y {
            if cy + rh <= h {
                return;
            }
        }
        // Put the cursor's row at the bottom of the view.
        self.scroll_row_to(c.line, row_px, h - rh, window);
    }

    /// Scrolls so the row `row_px` into `line` sits `need` pixels under the
    /// top of the view, walking back over the lines above it.
    fn scroll_row_to(
        &mut self,
        line: usize,
        row_px: Pixels,
        mut need: Pixels,
        window: &mut Window,
    ) {
        let mut line = line;
        let mut px_in_line = row_px;
        loop {
            if px_in_line >= need {
                self.scroll_line = line;
                self.scroll_px = px_in_line - need;
                break;
            }
            need -= px_in_line;
            if line == 0 {
                self.scroll_line = 0;
                self.scroll_px = px(0.);
                break;
            }
            line -= 1;
            px_in_line = self.height_of(window, line);
        }
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
        self.after_edit(window, cx);
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
            self.after_edit(window, cx);
        }
    }

    /// Inserts `s` at every cursor, replacing what each has selected.
    fn insert_all(&mut self, s: &str, kind: EditKind, window: &mut Window, cx: &mut Context<Self>) {
        if self.buf.insert_all(s, kind) {
            self.after_edit(Some(window), cx);
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
            self.after_edit(Some(window), cx);
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
            self.after_edit(Some(window), cx);
        }
    }

    fn after_edit(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        // An edit never happens out of sight.
        self.unfold_at_cursor();
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
            self.after_edit(Some(window), cx);
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
                // A slightly stale decoration set is better than none while typing.
                editor.highlights = Arc::new(result.0);
                let widgets = result.6;
                editor
                    .widget_heights
                    .retain(|first, _| widgets.iter().any(|(l, _)| l.start == *first));
                // Widgets change the heights above the cursor: keep it on screen.
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
                // Unlike the decorations, a stale set of foldables is worse
                // than none: resolving the folds against another text's
                // lines would drop them. Only the parse of the current text
                // settles them, and a record waiting for the first parse
                // waits for that one.
                if editor.generation == generation {
                    editor.foldables = Arc::new(result.8);
                    if let Some(froms) = editor.pending_folds.take() {
                        editor.folds = fold::Folds::from_record(&editor.foldables, froms);
                    }
                    editor.refresh_folds();
                }
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

    // ----- movement ------------------------------------------------------------

    /// Moves the primary cursor to `offset`, extending its selection when
    /// `select`; other cursors are dropped.
    fn move_to(
        &mut self,
        offset: usize,
        select: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.buf.collapse_cursors();
        self.buf.head = self.buf.clamp_offset(offset);
        if !select {
            self.buf.anchor = self.buf.head;
        }
        self.buf.marked = None;
        self.buf.forget_planted();
        // A cursor move refreshes a popup that is open, and closes it once
        // the cursor has left the query.
        self.update_suggest(false, cx);
        self.reveal_cursor(window);
        cx.notify();
    }

    /// Moves every cursor to where `target` sends it (its new head and
    /// goal x), extending the selections when `select`.
    fn move_each(
        &mut self,
        select: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
        mut target: impl FnMut(&Self, &mut Window, Sel) -> (usize, Option<f32>),
    ) {
        let cursors = self.buf.cursors();
        let mut sels = Vec::with_capacity(cursors.sels.len());
        for s in &cursors.sels {
            let (head, goal) = target(self, window, *s);
            let head = self.buf.clamp_offset(head);
            sels.push(Sel {
                anchor: if select { s.anchor } else { head },
                head,
                goal,
            });
        }
        self.buf.set_cursors(Cursors {
            sels,
            primary: cursors.primary,
        });
        self.buf.marked = None;
        // A cursor move refreshes a popup that is open, and closes it once
        // the cursor has left the query.
        self.update_suggest(false, cx);
        self.reveal_cursor(window);
        cx.notify();
    }

    fn horizontal(
        &mut self,
        left: bool,
        select: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_each(select, window, cx, |this, _, s| {
            let sel = s.range();
            let target = if !select && !s.is_empty() {
                // A plain arrow collapses the selection to its edge.
                if left {
                    sel.start
                } else {
                    sel.end
                }
            } else if left {
                this.buf.prev_char_boundary(s.head)
            } else {
                this.buf.next_char_boundary(s.head)
            };
            (target, None)
        });
    }

    /// Moves `delta` wrapped rows up (negative) or down, keeping the goal x.
    fn vertical(
        &mut self,
        delta: isize,
        select: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_each(select, window, cx, |this, window, s| {
            this.vertical_target(window, s.head, s.goal, delta)
        });
    }

    /// Where `delta` rows up or down from `head` lands, and the goal x to
    /// keep for the next vertical move.
    fn vertical_target(
        &self,
        window: &mut Window,
        head: usize,
        goal_x: Option<f32>,
        delta: isize,
    ) -> (usize, Option<f32>) {
        let lh = self.layout.line_height;
        let c = self.buf.cursor_at(head);
        let Some(shaped) = self.shape(window, c.line).filter(|_| lh > px(0.)) else {
            // No layout yet: move by buffer lines.
            let line = (c.line as isize + delta).clamp(0, self.line_count() as isize - 1) as usize;
            let col = self.buf.clamp_col(line, c.col);
            return (self.buf.line_starts()[line] + col, None);
        };
        let (start_row, x) = shaped.row_and_x(c.col);
        let goal = goal_x.map(px).unwrap_or(x);
        let mut line = c.line;
        let mut row = start_row;
        let mut rows = shaped.rows();
        let mut remaining = delta;
        // The lines a fold hides are not on screen, so a vertical step
        // passes over them.
        let above = |l: usize| (0..l).rev().find(|&c| !self.line_hidden(c));
        let below = |l: usize| (l + 1..self.line_count()).find(|&c| !self.line_hidden(c));
        while remaining != 0 {
            if remaining < 0 {
                if row > 0 {
                    row -= 1;
                } else if let Some(prev) = above(line) {
                    line = prev;
                    rows = self.rows_of(window, line);
                    row = rows - 1;
                } else {
                    break;
                }
                remaining += 1;
            } else {
                if row + 1 < rows {
                    row += 1;
                } else if let Some(next) = below(line) {
                    line = next;
                    rows = self.rows_of(window, line);
                    row = 0;
                } else {
                    break;
                }
                remaining -= 1;
            }
        }
        let target = if line == c.line && row == start_row {
            // Nowhere to go: the first row goes to the start, the last to the end.
            if delta < 0 {
                self.buf.line_starts()[line]
            } else {
                self.buf.line_range(line).end
            }
        } else {
            let shaped = if line == c.line {
                shaped
            } else {
                match self.shape(window, line) {
                    Some(s) => s,
                    None => return (head, goal_x),
                }
            };
            let col = self.buf.clamp_col(line, shaped.index_at(goal, row));
            self.buf.line_starts()[line] + col
        };
        (target, Some(f32::from(goal)))
    }

    fn row_boundary(
        &mut self,
        end: bool,
        select: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_each(select, window, cx, |this, window, s| {
            let c = this.buf.cursor_at(s.head);
            let target = match this.shape(window, c.line) {
                Some(shaped) if this.layout.line_height > px(0.) => {
                    let (row, _) = shaped.row_and_x(c.col);
                    let x = if end { px(f32::MAX / 4.) } else { px(0.) };
                    let col = this.buf.clamp_col(c.line, shaped.index_at(x, row));
                    this.buf.line_starts()[c.line] + col
                }
                _ => {
                    if end {
                        this.buf.line_range(c.line).end
                    } else {
                        this.buf.line_starts()[c.line]
                    }
                }
            };
            (target, None)
        });
    }

    /// `editor:add-cursor-above` and `below`: a new cursor one row above
    /// the topmost (or below the bottommost), at the same x.
    pub fn add_cursor(&mut self, above: bool, window: &mut Window, cx: &mut Context<Self>) {
        let cursors = self.buf.cursors();
        let from = if above {
            cursors.sels[0]
        } else {
            cursors.sels[cursors.sels.len() - 1]
        };
        let (head, goal) =
            self.vertical_target(window, from.head, from.goal, if above { -1 } else { 1 });
        if self.buf.cursor_at(head).line == self.buf.cursor_at(from.head).line {
            return;
        }
        let mut sels = cursors.sels;
        sels.push(Sel {
            anchor: head,
            head,
            goal,
        });
        self.buf.set_cursors(Cursors {
            sels,
            primary: cursors.primary,
        });
        self.reveal_cursor(window);
        cx.notify();
    }

    fn add_cursor_above(
        &mut self,
        _: &AddCursorAbove,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_cursor(true, window, cx);
    }

    fn add_cursor_below(
        &mut self,
        _: &AddCursorBelow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_cursor(false, window, cx);
    }

    fn page(&mut self, down: bool, select: bool, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.visible_rows() as isize;
        let lh = self.layout.line_height;
        let dy = lh * rows as f32;
        self.scroll_by(if down { dy } else { -dy }, window, cx);
        self.vertical(if down { rows } else { -rows }, select, window, cx);
    }

    fn move_left(&mut self, _: &MoveLeft, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(f) = self.active_field() {
            f.left();
            cx.notify();
            return;
        }
        self.horizontal(true, false, window, cx);
    }

    fn move_right(&mut self, _: &MoveRight, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(f) = self.active_field() {
            f.right();
            cx.notify();
            return;
        }
        self.horizontal(false, false, window, cx);
    }

    fn select_left(&mut self, _: &SelectLeft, window: &mut Window, cx: &mut Context<Self>) {
        self.horizontal(true, true, window, cx);
    }

    fn select_right(&mut self, _: &SelectRight, window: &mut Window, cx: &mut Context<Self>) {
        self.horizontal(false, true, window, cx);
    }

    fn move_up(&mut self, _: &MoveUp, window: &mut Window, cx: &mut Context<Self>) {
        if self.suggest_step(-1, cx) {
            return;
        }
        // Leaving the line forgets the closers auto-pairing planted on it:
        // coming back and typing one writes it (Specification.md,
        // "Observed auto-pair behavior").
        self.buf.forget_planted();
        self.vertical(-1, false, window, cx);
    }

    fn move_down(&mut self, _: &MoveDown, window: &mut Window, cx: &mut Context<Self>) {
        if self.suggest_step(1, cx) {
            return;
        }
        self.buf.forget_planted();
        self.vertical(1, false, window, cx);
    }

    fn select_up(&mut self, _: &SelectUp, window: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-1, true, window, cx);
    }

    fn select_down(&mut self, _: &SelectDown, window: &mut Window, cx: &mut Context<Self>) {
        self.vertical(1, true, window, cx);
    }

    fn move_word_left(&mut self, _: &MoveWordLeft, window: &mut Window, cx: &mut Context<Self>) {
        self.move_each(false, window, cx, |this, _, s| {
            (this.buf.word_left(s.head), None)
        });
    }

    fn move_word_right(&mut self, _: &MoveWordRight, window: &mut Window, cx: &mut Context<Self>) {
        self.move_each(false, window, cx, |this, _, s| {
            (this.buf.word_right(s.head), None)
        });
    }

    fn select_word_left(
        &mut self,
        _: &SelectWordLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_each(true, window, cx, |this, _, s| {
            (this.buf.word_left(s.head), None)
        });
    }

    fn select_word_right(
        &mut self,
        _: &SelectWordRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_each(true, window, cx, |this, _, s| {
            (this.buf.word_right(s.head), None)
        });
    }

    fn page_up(&mut self, _: &PageUp, window: &mut Window, cx: &mut Context<Self>) {
        if self.suggest_step(-(crate::suggest::VISIBLE as isize), cx) {
            return;
        }
        self.page(false, false, window, cx);
    }

    fn page_down(&mut self, _: &PageDown, window: &mut Window, cx: &mut Context<Self>) {
        if self.suggest_step(crate::suggest::VISIBLE as isize, cx) {
            return;
        }
        self.page(true, false, window, cx);
    }

    fn select_page_up(&mut self, _: &SelectPageUp, window: &mut Window, cx: &mut Context<Self>) {
        self.page(false, true, window, cx);
    }

    fn select_page_down(
        &mut self,
        _: &SelectPageDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.page(true, true, window, cx);
    }

    fn line_start(&mut self, _: &LineStart, window: &mut Window, cx: &mut Context<Self>) {
        if self.suggest_step(isize::MIN, cx) {
            return;
        }
        if let Some(f) = self.active_field() {
            f.home();
            cx.notify();
            return;
        }
        self.row_boundary(false, false, window, cx);
    }

    fn line_end(&mut self, _: &LineEnd, window: &mut Window, cx: &mut Context<Self>) {
        if self.suggest_step(isize::MAX, cx) {
            return;
        }
        if let Some(f) = self.active_field() {
            f.end();
            cx.notify();
            return;
        }
        self.row_boundary(true, false, window, cx);
    }

    fn select_line_start(
        &mut self,
        _: &SelectLineStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.row_boundary(false, true, window, cx);
    }

    fn select_line_end(&mut self, _: &SelectLineEnd, window: &mut Window, cx: &mut Context<Self>) {
        self.row_boundary(true, true, window, cx);
    }

    fn document_start(&mut self, _: &DocumentStart, window: &mut Window, cx: &mut Context<Self>) {
        self.buf.goal = None;
        self.move_to(0, false, window, cx);
    }

    fn document_end(&mut self, _: &DocumentEnd, window: &mut Window, cx: &mut Context<Self>) {
        self.buf.goal = None;
        self.move_to(self.buf.len(), false, window, cx);
    }

    fn select_document_start(
        &mut self,
        _: &SelectDocumentStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.buf.goal = None;
        self.move_to(0, true, window, cx);
    }

    fn select_document_end(
        &mut self,
        _: &SelectDocumentEnd,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.buf.goal = None;
        self.move_to(self.buf.len(), true, window, cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        // An open field takes the keys the way backspace and delete give
        // them to it. It carries a caret and no selection, so there is
        // nothing for Ctrl+A to take; what it must not do is select the
        // document behind the field, which the next Ctrl+X would delete.
        if self.field_kind().is_some() {
            return;
        }
        self.buf.select_all();
        cx.notify();
    }

    // ----- editing actions -------------------------------------------------------

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        // A properties field has the pill gesture behind it, so it takes the
        // press through its own path.
        if self.property_field.is_some() {
            self.backspace_property_field(window, cx);
            return;
        }
        if let Some(f) = self.active_field() {
            let _ = f.backspace(None);
            cx.notify();
            return;
        }
        let kind = if self.selection().is_empty() {
            EditKind::Backspace
        } else {
            EditKind::Other
        };
        // Backspace between an empty pair takes both characters.
        let opts = self.auto_pair();
        self.delete_all(
            move |buf, at| autopair::backspace_range(buf.text(), at, opts),
            kind,
            window,
            cx,
        );
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(f) = self.active_field() {
            f.delete();
            cx.notify();
            return;
        }
        let kind = if self.selection().is_empty() {
            EditKind::Delete
        } else {
            EditKind::Other
        };
        self.delete_all(|buf, at| at..buf.next_char_boundary(at), kind, window, cx);
    }

    fn delete_word_left(
        &mut self,
        _: &DeleteWordLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_all(|buf, at| buf.word_left(at)..at, EditKind::Other, window, cx);
    }

    fn delete_word_right(
        &mut self,
        _: &DeleteWordRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_all(
            |buf, at| at..buf.word_right(at),
            EditKind::Other,
            window,
            cx,
        );
    }

    /// Enter: a newline that continues list and quote markers and the
    /// numbering of ordered lists, and clears or outdents an empty item.
    /// With several cursors, or a selection, a plain newline at each.
    fn newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        if self.accept_suggestion(Accept::Whole, window, cx) {
            return;
        }
        // Enter in a properties field commits it, as Obsidian's does, and in
        // a list's trailing input leaves that input focused for the next
        // item.
        if self.property_field.is_some() {
            self.enter_property_field(window, cx);
            return;
        }
        // In a table cell it commits and drops into the cell below, or just
        // commits on the last row.
        if self.table_field.is_some() {
            if !self.step_table_cell(0, 1, window, cx) {
                self.commit_table_field(Some(window), cx, None);
            }
            return;
        }
        if self.buf.newline(self.tab_size, self.smart_indent_list) {
            self.after_edit(Some(window), cx);
        }
    }

    /// Shift+Enter: a plain newline.
    fn newline_plain(&mut self, _: &NewlinePlain, window: &mut Window, cx: &mut Context<Self>) {
        if self.accept_suggestion(Accept::Typed, window, cx) {
            return;
        }
        self.insert_all("\n", EditKind::Other, window, cx);
    }

    /// Tab: indents list items and selected lines by one step (renumbering
    /// the ordered lists it touches), and inserts the step anywhere else.
    fn tab(&mut self, _: &Tab, window: &mut Window, cx: &mut Context<Self>) {
        if self.accept_suggestion(Accept::Inside, window, cx) {
            return;
        }
        // Tab walks the cells of a table, committing each as it leaves it.
        if self.table_field.is_some() && self.step_table_cell(1, 0, window, cx) {
            return;
        }
        // Tab commits a key field and moves to that property's value, as
        // Obsidian's key input does (`enterkeyhint="next"`). From a value
        // field it just commits, since there is nothing after it.
        if let Some(field) = self.property_field.clone() {
            self.commit_property_field(Some(window), cx);
            // The key the commit left behind: what was typed, for both a
            // rename and a newly named row.
            let key = field.text.trim().to_string();
            if field.part == Part::Key && !key.is_empty() {
                if let Some(row) = self.property_rows().into_iter().find(|r| r.key == key) {
                    self.property_field = Some(Field::new(
                        Target::Existing(key),
                        Part::Value,
                        row.edit_text,
                    ));
                }
            }
            cx.notify();
            return;
        }
        if self
            .buf
            .indent(self.use_tab, self.tab_size, self.smart_indent_list)
        {
            self.after_edit(Some(window), cx);
        }
    }

    /// Shift+Tab: removes one indentation step from the selected lines,
    /// renumbering the ordered lists it touches.
    fn outdent(&mut self, _: &Outdent, window: &mut Window, cx: &mut Context<Self>) {
        if self.table_field.is_some() && self.step_table_cell(-1, 0, window, cx) {
            return;
        }
        if self
            .buf
            .outdent(self.use_tab, self.tab_size, self.smart_indent_list)
        {
            self.after_edit(Some(window), cx);
        }
    }

    fn undo(&mut self, _: &Undo, window: &mut Window, cx: &mut Context<Self>) {
        if self.buf.undo() {
            self.after_edit(Some(window), cx);
        }
    }

    fn redo(&mut self, _: &Redo, window: &mut Window, cx: &mut Context<Self>) {
        if self.buf.redo() {
            self.after_edit(Some(window), cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if self.field_kind().is_some() {
            return;
        }
        let ranges = self.buf.copy_ranges();
        if ranges.iter().any(|r| !r.is_empty()) {
            cx.write_to_clipboard(ClipboardItem::new_string(self.buf.clipboard_text(&ranges)));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if self.field_kind().is_some() {
            return;
        }
        let ranges = self.buf.copy_ranges();
        if ranges.iter().any(|r| !r.is_empty()) {
            cx.write_to_clipboard(ClipboardItem::new_string(self.buf.clipboard_text(&ranges)));
            let edits = ranges.into_iter().map(|r| (r, String::new())).collect();
            self.edit_all(edits, EditKind::Other, Some(window), cx);
        }
    }

    /// Paste at every cursor; a clipboard with one line per cursor gives
    /// each its own line, the way CodeMirror pastes a multi-cursor copy.
    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        // Into the open field, which drops the newlines a row cannot hold.
        if let Some(field) = self.active_field() {
            if let Some(text) = item.text() {
                field.insert(&text);
                cx.notify();
            }
            return;
        }

        // Attachment paste: image or external file paths
        if let Some(state) = self.suggest_vault.clone() {
            if !self.name.is_empty() {
                let image_entry = item.entries.iter().find_map(|entry| match entry {
                    gpui::ClipboardEntry::Image(img) => Some(img),
                    _ => None,
                });
                if let Some(img) = image_entry {
                    if self.paste_image_attachment(img, &state, window, cx) {
                        return;
                    }
                }

                let paths_entry = item.entries.iter().find_map(|entry| match entry {
                    gpui::ClipboardEntry::ExternalPaths(paths) => Some(paths),
                    _ => None,
                });
                if let Some(paths) = paths_entry {
                    if self.paste_files_attachment(&paths.0, &state, window, cx) {
                        return;
                    }
                }
            }
        }

        let Some(text) = item.text() else {
            return;
        };
        if self.buf.paste(&text) {
            self.after_edit(Some(window), cx);
        }
    }

    fn paste_image_attachment(
        &mut self,
        img: &gpui::Image,
        state: &Arc<VaultState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if img.bytes.is_empty() {
            return false;
        }
        let folder = silica_vault::ops::attachment_folder(
            state.app().attachment_folder_path.as_deref(),
            &self.name,
        );
        let timestamp = chrono::Local::now().format("%Y%m%d%H%M%S");
        let stem = format!("Pasted image {timestamp}");
        let ext = match img.format {
            gpui::ImageFormat::Jpeg => "jpg",
            gpui::ImageFormat::Gif => "gif",
            gpui::ImageFormat::Webp => "webp",
            gpui::ImageFormat::Svg => "svg",
            gpui::ImageFormat::Bmp => "bmp",
            gpui::ImageFormat::Tiff => "tiff",
            _ => "png",
        };
        let target = state.vault().available_path(&folder, &stem, ext);
        match state.save_attachment(&target, &img.bytes) {
            Ok(changes) => {
                let resolver = state.vault().link_resolver();
                let written = silica_cache::links::link_path(&resolver, &target, &self.name, self.link_options.format);
                let embed = if self.link_options.markdown_links {
                    format!("![]({})", written.replace(' ', "%20"))
                } else {
                    format!("![[{written}]]")
                };
                if self.buf.paste(&embed) {
                    self.after_edit(Some(window), cx);
                }
                cx.emit(EditorEvent::FilesChanged(changes));
                true
            }
            Err(e) => {
                cx.emit(EditorEvent::Reported(e.into()));
                false
            }
        }
    }

    fn paste_files_attachment(
        &mut self,
        paths: &[std::path::PathBuf],
        state: &Arc<VaultState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if paths.is_empty() {
            return false;
        }
        let folder = silica_vault::ops::attachment_folder(
            state.app().attachment_folder_path.as_deref(),
            &self.name,
        );
        let vault_root = state.vault().root().to_path_buf();
        let mut embeds = Vec::new();
        let mut all_changes = crate::workspace::Changes::default();

        for path in paths {
            if !path.is_file() {
                continue;
            }
            let target = if let Ok(rel) = path.strip_prefix(&vault_root) {
                rel.to_string_lossy().replace('\\', "/")
            } else {
                let filename = match path.file_name().and_then(|n| n.to_str()) {
                    Some(name) => name,
                    None => continue,
                };
                let (stem, ext) = match filename.rsplit_once('.') {
                    Some((s, e)) if !s.is_empty() => (s, e),
                    _ => (filename, ""),
                };
                let target = state.vault().available_path(&folder, stem, ext);
                let bytes = match std::fs::read(path) {
                    Ok(b) => b,
                    Err(e) => {
                        cx.emit(EditorEvent::Reported(format!("Could not read file {}: {e}", path.display()).into()));
                        continue;
                    }
                };
                match state.save_attachment(&target, &bytes) {
                    Ok(changes) => {
                        all_changes.extend(changes);
                        target
                    }
                    Err(e) => {
                        cx.emit(EditorEvent::Reported(e.into()));
                        continue;
                    }
                }
            };

            let resolver = state.vault().link_resolver();
            let written = silica_cache::links::link_path(&resolver, &target, &self.name, self.link_options.format);
            let embed = if self.link_options.markdown_links {
                format!("![]({})", written.replace(' ', "%20"))
            } else {
                format!("![[{written}]]")
            };
            embeds.push(embed);
        }

        if embeds.is_empty() {
            return false;
        }

        let combined = embeds.join("\n");
        if self.buf.paste(&combined) {
            self.after_edit(Some(window), cx);
        }
        if !all_changes.files.is_empty() {
            cx.emit(EditorEvent::FilesChanged(all_changes));
        }
        true
    }

    fn handle_drop(
        &mut self,
        paths: &gpui::ExternalPaths,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.name.is_empty() {
            return;
        }
        let Some(state) = self.suggest_vault.clone() else {
            return;
        };
        self.paste_files_attachment(&paths.0, &state, window, cx);
    }

    // ----- find and replace ------------------------------------------------------

    /// Opens the panel (Ctrl+F, or Ctrl+H with the replace row). The
    /// selection, when it sits on one line, becomes the query; an explicit
    /// `query` wins; otherwise the previous query stays.
    pub fn open_find(
        &mut self,
        query: &str,
        replace: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find.open = true;
        if replace {
            self.find.replace_shown = true;
        }
        let sel = self.selection();
        let selected = (sel.start != sel.end && !self.buf.text()[sel.clone()].contains('\n'))
            .then(|| self.buf.text()[sel].to_string());
        let text = if !query.is_empty() {
            Some(query.to_string())
        } else {
            selected
        };
        match text {
            Some(t) => self.find.query.update(cx, |q, cx| q.set_text(t, cx)),
            None => self.refresh_matches(cx),
        }
        // Obsidian's panel opens on the query whichever command opened
        // it; `replace` only decides whether the replacement row shows.
        // The whole query is selected the way a browser input's is, so the
        // first keystroke replaces the last note's query instead of
        // appending to it, and the old query is still there to keep.
        self.find.query.update(cx, |q, cx| q.select_all(cx));
        window.focus(&self.find.query.focus_handle(cx), cx);
        cx.notify();
    }

    /// Tab and Shift+Tab in the find panel move between the query and the
    /// replacement. With two fields each key does the same thing, and
    /// without the replacement row there is nowhere else to go.
    fn focus_other_field(
        &mut self,
        _: &FocusOtherField,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.find.replace_shown {
            return;
        }
        let on_query = self.find.query.focus_handle(cx).is_focused(window);
        let field = if on_query {
            &self.find.replacement
        } else {
            &self.find.query
        };
        // Tabbing into a field selects what it holds, as it does between
        // two HTML inputs.
        field.update(cx, |f, cx| f.select_all(cx));
        window.focus(&field.focus_handle(cx), cx);
        cx.notify();
    }

    /// Escape: closes the autocompletion popup first, then abandons an open
    /// field, then drops extra cursors, then closes the panel.
    fn close_find(&mut self, _: &CloseFind, window: &mut Window, cx: &mut Context<Self>) {
        if self.suggest.is_some() {
            self.close_suggest(cx);
        } else if self.cancel_table_field(cx) || self.cancel_property_field(cx) {
        } else if !self.buf.extra.is_empty() && self.focus_handle.is_focused(window) {
            self.buf.collapse_cursors();
            cx.notify();
        } else if self.find.open {
            self.find.open = false;
            window.focus(&self.focus_handle, cx);
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    /// Flips match case or regex and searches again.
    fn toggle_find_option(&mut self, regex: bool, cx: &mut Context<Self>) {
        if regex {
            self.find.opts.regex = !self.find.opts.regex;
        } else {
            self.find.opts.match_case = !self.find.opts.match_case;
        }
        self.query_changed(cx);
    }

    /// Recomputes the matches for the current query (after an edit, or
    /// when the query changes); the selected match is kept where it still is.
    fn refresh_matches(&mut self, cx: &mut Context<Self>) {
        if !self.find.open {
            return;
        }
        let query = self.find.query.read(cx).text().to_string();
        self.find.matches = find::find_matches(self.buf.text(), &query, self.find.opts);
        let sel = self.selection();
        self.find.current = self.find.matches.iter().position(|m| *m == sel);
    }

    /// Typing in the query selects the first match from the cursor on.
    fn query_changed(&mut self, cx: &mut Context<Self>) {
        if !self.find.open {
            return;
        }
        self.refresh_matches(cx);
        let from = self.selection().start;
        if let Some(ix) = find::next_from(&self.find.matches, from) {
            self.select_match(ix, cx);
        } else {
            self.find.current = None;
            cx.notify();
        }
    }

    fn select_match(&mut self, ix: usize, cx: &mut Context<Self>) {
        let m = self.find.matches[ix].clone();
        self.buf.collapse_cursors();
        self.buf.anchor = m.start;
        self.buf.head = m.end;
        self.find.current = Some(ix);
        self.buf.goal = None;
        self.pending_reveal = true;
        cx.notify();
    }

    /// Enter and Shift+Enter in the query: the next or previous match.
    fn find_step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let sel = self.selection();
        let ix = if forward {
            let from = if self
                .find
                .current
                .is_some_and(|c| self.find.matches[c] == sel)
            {
                sel.end
            } else {
                sel.start
            };
            find::next_from(&self.find.matches, from)
        } else {
            find::previous_from(&self.find.matches, sel.start)
        };
        match ix {
            Some(ix) => self.select_match(ix, cx),
            None => self.find.current = None,
        }
    }

    fn find_next(&mut self, _: &FindNext, _: &mut Window, cx: &mut Context<Self>) {
        self.find_step(true, cx);
    }

    fn find_previous(&mut self, _: &FindPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.find_step(false, cx);
    }

    /// Find all (Alt+Enter in the query): every match becomes a selection,
    /// the panel closes and the editor takes focus, so typing edits them
    /// all at once.
    fn find_all(&mut self, _: &FindAll, window: &mut Window, cx: &mut Context<Self>) {
        self.find_all_matches(window, cx);
    }

    /// [`Editor::find_all`] for the command line (`--find-all`).
    pub fn find_all_matches(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // A query set in the same update has not reported its change yet.
        self.refresh_matches(cx);
        if self.find.matches.is_empty() {
            return;
        }
        let sels: Vec<Sel> = self
            .find
            .matches
            .iter()
            .map(|m| Sel {
                anchor: m.start,
                head: m.end,
                goal: None,
            })
            .collect();
        let primary = self.find.current.unwrap_or(0).min(sels.len() - 1);
        self.buf.set_cursors(Cursors { sels, primary });
        self.find.open = false;
        window.focus(&self.focus_handle, cx);
        self.pending_reveal = true;
        cx.notify();
    }

    /// What replaces the match at `m` under the current options.
    fn replacement_for(&self, m: &Range<usize>, cx: &Context<Self>) -> String {
        let with = self.find.replacement.read(cx).text().to_string();
        let query = self.find.query.read(cx).text();
        match find::compile(query, self.find.opts) {
            Some(re) => find::replacement_for(self.buf.text(), m, &re, self.find.opts, &with),
            None => with,
        }
    }

    /// Replaces the selected match and moves to the next one; with no match
    /// selected, only moves.
    fn replace_next(&mut self, cx: &mut Context<Self>) {
        let sel = self.selection();
        if let Some(ix) = self.find.current.filter(|&c| self.find.matches[c] == sel) {
            let m = self.find.matches[ix].clone();
            let with = self.replacement_for(&m, cx);
            self.edit_many_no_window(vec![(m.clone(), with.clone())], m.start + with.len(), cx);
        }
        self.find_step(true, cx);
    }

    /// Replaces every match in one undo step.
    fn replace_all(&mut self, cx: &mut Context<Self>) {
        if self.find.matches.is_empty() {
            return;
        }
        let edits: Vec<(Range<usize>, String)> = self
            .find
            .matches
            .iter()
            .map(|m| (m.clone(), self.replacement_for(m, cx)))
            .collect();
        // The cursor lands after the last replacement in the new text.
        let last_end = edits.last().map(|(r, _)| r.end).unwrap_or(0);
        let cursor = shifted(last_end, &edits);
        self.edit_many_no_window(edits, cursor, cx);
    }

    /// `edit_many` from a field's event, where no window is at hand: the
    /// cursor is revealed on the next frame.
    fn edit_many_no_window(
        &mut self,
        edits: Vec<(Range<usize>, String)>,
        cursor: usize,
        cx: &mut Context<Self>,
    ) {
        self.buf
            .apply_edits(edits, Cursors::at(cursor), EditKind::Other);
        self.after_edit(None, cx);
    }

    // ----- mouse ---------------------------------------------------------------

    /// The visible line under a window point, shaped as painted, and the
    /// display byte nearest the point.
    /// Reopens, refreshes or closes the autocompletion popup for the text
    /// before the cursor.
    ///
    /// `opening` is set after an edit. A bare cursor move only refreshes a
    /// popup that is already open, so clicking into the middle of a link
    /// that is already written does not pop one up.
    fn update_suggest(&mut self, opening: bool, cx: &mut Context<Self>) {
        if !opening && self.suggest.is_none() {
            return;
        }
        // One query, one cursor: a suggestion inserted at several at once
        // would be several different links.
        if self.cursor_count() > 1 || self.property_field.is_some() {
            self.close_suggest(cx);
            return;
        }
        let Some(state) = self.suggest_vault.clone() else {
            self.close_suggest(cx);
            return;
        };
        let cursor = self.cursor();
        let line = self.buf.line_text(cursor.line).to_string();
        let Some(t) = suggest::trigger(&line, cursor.col) else {
            self.close_suggest(cx);
            return;
        };
        let items = self.offer(&state, &t);
        if items.is_empty() {
            self.close_suggest(cx);
            return;
        }
        self.suggest = Some(SuggestState {
            line: cursor.line,
            range: t.range.clone(),
            items,
            selected: 0,
        });
        cx.notify();
    }

    /// The rows the vault offers for `t`, read under its locks.
    fn offer(&self, state: &Arc<VaultState>, t: &suggest::Triggered<'_>) -> Vec<Suggestion> {
        let vault = state.vault();
        let cache = state.cache();
        let show_unsupported = state.app().show_unsupported_files.unwrap_or(false);
        // Only a `#^` query asks for a note's text, so this costs nothing
        // on the common `[[` keystroke.
        let texts = |path: &str| state.texts().get(path).map(|t| t.to_string());
        suggest::suggestions(
            t,
            &suggest::Vault {
                files: vault.files(),
                cache: &cache,
                source: self.name.as_ref(),
                show_unsupported,
                text: &texts,
            },
        )
    }

    pub(crate) fn close_suggest(&mut self, cx: &mut Context<Self>) {
        if self.suggest.take().is_some() {
            cx.notify();
        }
    }

    /// Moves the popup's selection, if it is open. Up, down, the page keys
    /// and Home and End all belong to the popup while it is, which is what
    /// Obsidian's popup binds.
    fn suggest_step(&mut self, delta: isize, cx: &mut Context<Self>) -> bool {
        let Some(s) = self.suggest.as_mut() else {
            return false;
        };
        match delta {
            isize::MIN => s.select_first(),
            isize::MAX => s.select_last(),
            delta => s.step(delta),
        }
        cx.notify();
        true
    }

    /// Accepts the selected row, if the popup is open.
    fn accept_suggestion(
        &mut self,
        accept: Accept,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(ix) = self.suggest.as_ref().map(|s| s.selected) else {
            return false;
        };
        self.accept_suggestion_at(ix, accept, window, cx)
    }

    /// Writes the row at `ix` into the document and puts the cursor where
    /// the accept key leaves it. The trigger is read from the line again so
    /// the text that is replaced is the text that is there now.
    pub(crate) fn accept_suggestion_at(
        &mut self,
        ix: usize,
        accept: Accept,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(state) = self.suggest_vault.clone() else {
            return false;
        };
        let Some((item, at, end)) = self.suggest.as_ref().and_then(|s| {
            let item = s.items.get(ix).map(|row| row.item.clone())?;
            Some((item, s.line, s.range.end))
        }) else {
            return false;
        };
        // A block with no id of its own cannot be linked until one is
        // written. The id goes in first, into this note or into the other
        // one, and the link is only written if that succeeded.
        let (item, own_note) = match item {
            suggest::Item::Block {
                path,
                id: BlockId::New { end, placement },
            } => match self.write_block_id(&state, &path, end, placement, cx) {
                Some((id, own)) => (
                    suggest::Item::Block {
                        path,
                        id: BlockId::Existing(id),
                    },
                    own,
                ),
                None => {
                    self.close_suggest(cx);
                    return false;
                }
            },
            other => (other, None),
        };
        let line_start = self.buf.line_range(at).start;
        let line = self.buf.line_text(at).to_string();
        let Some(t) = suggest::trigger(&line, end) else {
            self.close_suggest(cx);
            return false;
        };
        let ins = {
            let vault = state.vault();
            let cache = state.cache();
            let show_unsupported = state.app().show_unsupported_files.unwrap_or(false);
            let texts = |path: &str| state.texts().get(path).map(|t| t.to_string());
            suggest::insert(
                &t,
                &item,
                accept,
                &suggest::Vault {
                    files: vault.files(),
                    cache: &cache,
                    source: self.name.as_ref(),
                    show_unsupported,
                    text: &texts,
                },
                &self.link_options,
            )
        };
        let range = line_start + t.range.start..line_start + t.range.end + ins.consume;
        let mut cursor = range.start + ins.cursor;
        self.suggest = None;
        // An id written into this very note is part of the same edit, so
        // one undo takes back the whole accept.
        let mut edits = vec![(range.clone(), ins.text)];
        if let Some(id_ins) = own_note {
            if id_ins.range.start <= range.start {
                let grew = id_ins.text.len() as isize - id_ins.range.len() as isize;
                cursor = (cursor as isize + grew).max(0) as usize;
            }
            edits.push((id_ins.range, id_ins.text));
        }
        // One undo step, and the cursor where the insertion asks for it:
        // after the link for Enter, inside the brackets for Tab, where
        // `after_edit` opens the popup again on what is left to type.
        self.edit_many(edits, cursor, None, window, cx);
        true
    }

    /// Writes an id onto the block a row points at, so the row can be
    /// linked to.
    ///
    /// Returns the id, and the edit to fold into this note's own when the
    /// block is in the note being edited — that case has to go through the
    /// buffer, or the next save would write the id straight back out.
    /// `None` when nothing could be written, and the accept is abandoned.
    fn write_block_id(
        &mut self,
        state: &Arc<VaultState>,
        path: &str,
        end: usize,
        placement: Placement,
        cx: &mut Context<Self>,
    ) -> Option<(String, Option<IdInsertion>)> {
        let target = if path.is_empty() {
            self.name.to_string()
        } else {
            path.to_string()
        };
        let own = target == self.name.as_ref();
        // The note being edited is planned against the buffer, which is
        // ahead of the disk; another note against its own bytes.
        let text = if own {
            self.buf.text().to_string()
        } else {
            state.texts().get(&target)?.to_string()
        };
        let id = {
            let cache = state.cache();
            let taken = cache.file(&target).map(|m| m.blocks.clone())?;
            blocks::new_id(self.id_seed(), &taken)
        };
        let nl = if self.buf.crlf() && own {
            "\r\n"
        } else {
            newline_of(&text)
        };
        // A block an id cannot be written onto is refused rather than
        // written to, so a link Silica writes always resolves.
        let Some(insertion) = blocks::plan_id(&text, end, placement, &id, nl) else {
            cx.emit(EditorEvent::Reported(
                format!("No id can be written onto that block in {target}").into(),
            ));
            return None;
        };
        if own {
            return Some((id, Some(insertion)));
        }
        match state.write_block_id(&target, &text, &insertion) {
            Ok(()) => Some((id, None)),
            Err(err) => {
                cx.emit(EditorEvent::Reported(err.into()));
                None
            }
        }
    }

    /// The heading the cursor's line is, as `#Heading` — the subpath
    /// "Bookmark heading under cursor..." records. `None` off a heading.
    pub fn heading_subpath_at_cursor(&self) -> Option<String> {
        let line = self.cursor().line as u32;
        let meta = silica_cache::index_markdown(self.buf.text());
        meta.headings
            .iter()
            .find(|h| h.position.start.line == line)
            .map(|h| format!("#{}", h.heading.trim()))
    }

    /// The id of the block under the cursor, written into the note first
    /// when it has none — what "Bookmark block under cursor..." does. The
    /// buffer, not the disk, is what the cursor is in, so the block is
    /// found in the buffer's own index and the id goes in as one edit the
    /// autosave writes. `None` on a block that takes no id (a list as a
    /// whole, a thematic break, the frontmatter), which is reported.
    pub fn block_id_at_cursor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let line = self.cursor().line as u32;
        let text = self.buf.text().to_string();
        let meta = silica_cache::index_markdown(&text);
        let section = meta
            .sections
            .iter()
            .rfind(|s| s.position.start.line <= line && line <= s.position.end.line)?;
        if let Some(id) = &section.id {
            return Some(id.clone());
        }
        let Some(placement) = Placement::of(section.kind) else {
            cx.emit(EditorEvent::Reported(
                "That block cannot take a block id".into(),
            ));
            return None;
        };
        let id = blocks::new_id(self.id_seed(), &meta.blocks);
        let nl = if self.buf.crlf() {
            "\r\n"
        } else {
            newline_of(&text)
        };
        let Some(insertion) =
            blocks::plan_id(&text, section.position.end.offset, placement, &id, nl)
        else {
            cx.emit(EditorEvent::Reported(
                "No id can be written onto that block".into(),
            ));
            return None;
        };
        let cursor = self.buf.head;
        self.edit_many(
            vec![(insertion.range.clone(), insertion.text.clone())],
            cursor,
            None,
            window,
            cx,
        );
        Some(id)
    }

    /// A seed for a fresh block id. The clock is the only randomness the
    /// workspace has, and an id only has to be unique within its note.
    fn id_seed(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
            ^ (self.buf.head as u64).wrapping_mul(0x9e37_79b9)
    }

    /// Where the popup is drawn: under the start of the query, in the
    /// editor's own coordinates.
    fn suggest_anchor(&self, state: &SuggestState) -> Option<Anchor> {
        let shaped = self.layout.cursor_line.clone()?;
        let (row, x) = shaped.row_and_x(state.range.start.min(state.range.end));
        let rh = shaped.row_height;
        let origin = self.layout.bounds.origin;
        let top = self.layout.cursor_top - origin.y + rh * row as f32;
        Some(Anchor {
            x: self.layout.content_x - origin.x + x,
            top,
            bottom: top + rh,
            width: self.layout.bounds.size.width,
            height: self.layout.bounds.size.height,
        })
    }

    /// Resolves a window point against the last frame's layout.
    fn display_hit(&self, window: &mut Window, position: Point<Pixels>) -> Option<Hit> {
        let layout = &self.layout;
        if layout.line_height <= px(0.) || layout.visible.is_empty() {
            return None;
        }
        let y = position.y - layout.bounds.origin.y;
        let x = position.x - layout.content_x;
        // A point above the first line or below the last still names a row,
        // so the caret has somewhere to go, but it is on no line of text:
        // `on_line` carries that down to the glyph.
        let (vl, row, on_line) = match layout
            .visible
            .iter()
            .find(|v| y >= v.top && y < v.top + v.height())
        {
            Some(v) => (*v, ((y - v.top) / v.row_height).floor() as usize, true),
            None if y < layout.visible[0].top => (layout.visible[0], 0, false),
            None => {
                let last = *layout.visible.last()?;
                (last, last.rows - 1, false)
            }
        };
        let shaped = self.shape(window, vl.line)?;
        let display = shaped.display_index_at(x, row);
        let glyph = if on_line {
            shaped.glyph_index_at(x, row)
        } else {
            None
        };
        Some(Hit {
            line: vl.line,
            shaped,
            display,
            glyph,
        })
    }

    /// The buffer offset under a window point, using the last frame's layout.
    fn offset_at_point(&self, window: &mut Window, position: Point<Pixels>) -> Option<usize> {
        let hit = self.display_hit(window, position)?;
        let col = self
            .buf
            .clamp_col(hit.line, hit.shaped.to_source(hit.display));
        Some(self.buf.line_starts()[hit.line] + col)
    }

    /// The link under source byte `col` of `line`. With `rendered_only`,
    /// only a link Live Preview shows rendered counts (a revealed one is
    /// being edited).
    fn link_at(&self, line: usize, col: usize, rendered_only: bool) -> Option<LinkTarget> {
        let spans = self.highlights.get(line)?;
        let touch = self.touches_on_line(line);
        let text = self.buf.line_text(line);
        for s in spans {
            if !(s.range.start <= col && col < s.range.end) {
                continue;
            }
            let revealed = preview::touches(&touch, &s.range);
            if rendered_only && revealed {
                continue;
            }
            let t = text.get(s.range.clone())?;
            let target = match s.kind {
                HighlightKind::Wikilink | HighlightKind::Embed => {
                    let inner = t
                        .trim_start_matches('!')
                        .trim_start_matches("[[")
                        .trim_end_matches("]]");
                    let body = inner.split('|').next().unwrap_or(inner);
                    LinkTarget::Internal(body.to_string())
                }
                HighlightKind::Link => {
                    // `[text](dest "title")`; reference links are not followed.
                    let Some(open) = t.rfind("](") else {
                        continue;
                    };
                    let dest = t[open + 2..].trim_end_matches(')');
                    let dest = dest.split_whitespace().next().unwrap_or(dest);
                    let dest = dest.trim_matches(|c| c == '<' || c == '>');
                    if silica_markdown::html::is_vault_destination(dest) {
                        LinkTarget::Internal(
                            silica_markdown::html::percent_decode(dest).into_owned(),
                        )
                    } else {
                        LinkTarget::External(dest.to_string())
                    }
                }
                HighlightKind::Url => {
                    LinkTarget::External(t.trim_matches(|c| c == '<' || c == '>').to_string())
                }
                HighlightKind::Tag => LinkTarget::Tag(t.trim_start_matches('#').to_string()),
                _ => continue,
            };
            return Some(target);
        }
        None
    }

    /// Flips the task marker whose `[` is the byte at `col` on `line`:
    /// `[ ]` becomes `[x]` and anything else becomes `[ ]`, as one undo
    /// step that leaves the selection where it was (the three bytes are
    /// replaced by three, so no offset moves).
    ///
    /// Both rendered views toggle through here — Live Preview from its own
    /// click, the reading view by way of the workspace — so a checkbox
    /// writes the same bytes whichever one was clicked. Returns false, and
    /// writes nothing, when those bytes are not a marker: a reading view
    /// built before the note changed can name a position that has moved.
    pub fn toggle_task_at(
        &mut self,
        line: usize,
        col: usize,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(&start) = self.buf.line_starts().get(line) else {
            return false;
        };
        let checked = match self.buf.line_text(line).as_bytes().get(col..col + 3) {
            Some([b'[', status, b']']) => *status != b' ',
            _ => return false,
        };
        let with = if checked { "[ ]" } else { "[x]" };
        let (head, anchor) = (self.buf.head, self.buf.anchor);
        if self.buf.edit_many(
            vec![(start + col..start + col + 3, with.to_string())],
            head,
            Some(anchor),
        ) {
            self.after_edit(window, cx);
        }
        true
    }

    /// Live Preview clicks on rendered constructs: a checkbox toggles, a
    /// link opens (Ctrl+click opens a link in Source mode too). Returns
    /// true when the click was consumed.
    fn click_rendered(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if event.click_count != 1 || event.modifiers.shift {
            return false;
        }
        let Some(hit) = self.display_hit(window, event.position) else {
            return false;
        };
        // The left margin, the blank space past the end of a row, and the
        // area above and below the document are on no construct: the click
        // places the caret and nothing more. Only what was drawn under the
        // pointer can be worked.
        let Some(d) = hit.glyph else {
            return false;
        };
        let (line, shaped) = (hit.line, &hit.shaped);
        if self.live_preview {
            if let Some((src, HighlightKind::Task)) = shaped.display.sub_at(d) {
                self.toggle_task_at(line, src.start, Some(window), cx);
                return true;
            }
        }
        if self.live_preview || event.modifiers.control {
            let col = shaped.to_source(d);
            if let Some(target) = self.link_at(line, col, !event.modifiers.control) {
                cx.emit(if event.modifiers.control {
                    EditorEvent::OpenLinkInNewTab(target)
                } else {
                    EditorEvent::OpenLink(target)
                });
                return true;
            }
        }
        false
    }

    // ------------------------------------------------------- properties

    /// The properties block of the note as it stands, or `None` when it has
    /// none.
    fn properties(&self) -> Option<props::Properties> {
        silica_cache::note_properties(self.buf.text())
    }

    /// The display rows of the block, with each key's assigned type.
    fn property_rows(&self) -> Vec<properties::Row> {
        let Some(block) = self.properties() else {
            return Vec::new();
        };
        let mut next_id = crate::properties::PROPERTY_ID_BASE;
        properties::rows(
            &block.entries,
            |key| props::type_of(key, &block.entries, &self.property_types),
            &mut next_id,
        )
    }

    fn property_type_of(&self, key: &str) -> props::PropertyType {
        match self.properties() {
            Some(block) => props::type_of(key, &block.entries, &self.property_types),
            None => props::PropertyType::Text,
        }
    }

    /// Plans `changes` against the block and applies them as one undo step,
    /// so a field commit is one `Ctrl+Z` away however many lines it moved.
    fn apply_property_changes(
        &mut self,
        changes: &[props::Change],
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        let Some(block) = self.properties() else {
            // No block yet: a new property creates one above the body.
            if let Some(props::Change::Set { key, value }) = changes.first() {
                let nl = if self.buf.crlf() { "\r\n" } else { "\n" };
                let text = props::new_block(key, value, nl);
                // The buffer holds no BOM (it travels in `self.buf.bom()` and is
                // put back on save), so the block goes at offset zero.
                self.edit(0..0, &text, EditKind::Other, window, cx);
            }
            return;
        };
        let Some(edits) = props::plan(self.buf.text(), &block, changes) else {
            return;
        };
        let ranges: Vec<(Range<usize>, String)> = edits
            .into_iter()
            .map(|e| (e.start..e.end, e.text))
            .collect();
        // Keep the cursor where it was, clamped: the block above it may have
        // grown or shrunk, and the caret lives in a field, not the document.
        let head = self.buf.head.min(self.buf.len());
        let after = Cursors {
            sels: vec![Sel::at(head)],
            primary: 0,
        };
        self.buf.apply_edits(ranges, after, EditKind::Other);
        self.after_edit(window, cx);
    }

    /// `markdown:add-metadata-property`: opens an unnamed row at the end of
    /// the block. Obsidian writes nothing to the file until the row is named
    /// (confirmed with a probe: running the command left the note untouched).
    pub fn add_property_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.property_action(PropertyAction::AddRow, window, cx);
    }

    /// `markdown:clear-metadata-properties`: removes every property, and with
    /// it the fences, as Obsidian does (`---\na: 1\n---\nbody\n` leaves
    /// `body\n`).
    pub fn clear_properties(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.property_field = None;
        self.selected_pill = None;
        let Some(block) = self.properties() else {
            return;
        };
        let changes = properties::clear_all(&block.entries);
        if !changes.is_empty() {
            self.apply_property_changes(&changes, Some(window), cx);
        }
    }

    /// `editor:toggle-fold-properties`.
    pub fn toggle_properties_fold(&mut self, cx: &mut Context<Self>) {
        self.properties_folded = !self.properties_folded;
        cx.notify();
    }

    /// What the reader did to a property row.
    fn property_action(
        &mut self,
        action: PropertyAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A click is not a Backspace: whatever pill one selected, it loses
        // the selection.
        self.selected_pill = None;
        match action {
            PropertyAction::Edit { target, part } => {
                self.property_menu = None;
                self.commit_property_field(Some(window), cx);
                // A value opens through `value_field`, which lands in the
                // empty input after the pills when the row holds a list:
                // opening a list whole would commit it back through the
                // joined-string round trip and split an item at its comma.
                let row = match (&target, part) {
                    (Target::Existing(key), Part::Value) => {
                        self.property_rows().into_iter().find(|r| &r.key == key)
                    }
                    _ => None,
                };
                self.property_field = Some(match row {
                    Some(row) => properties::value_field(&row),
                    None => {
                        let text = match (&target, part) {
                            (Target::Existing(key), Part::Key) => key.clone(),
                            _ => String::new(),
                        };
                        Field::new(target, part, text)
                    }
                });
                cx.notify();
            }
            PropertyAction::EditItem { key, index } => {
                self.property_menu = None;
                self.commit_property_field(Some(window), cx);
                // The commit above may have rewritten the block, so the row
                // the field is built from is read after it, not before.
                if let Some(row) = self.property_rows().into_iter().find(|r| r.key == key) {
                    self.property_field = Some(row.item_field(index.min(row.items.len())));
                }
                cx.notify();
            }
            PropertyAction::RemoveItem { key, index } => {
                self.property_field = None;
                if let Some(row) = self.property_rows().into_iter().find(|r| r.key == key) {
                    if let Some(change) = properties::remove_item(&row, index) {
                        self.apply_property_changes(&[change], Some(window), cx);
                    }
                }
            }
            PropertyAction::Toggle { key } => {
                let on = self
                    .property_rows()
                    .into_iter()
                    .find(|r| r.key == key)
                    .map(|r| matches!(r.value, properties::Display::Checkbox(true)))
                    .unwrap_or(false);
                self.apply_property_changes(
                    &[props::Change::Set {
                        key,
                        value: props::Value::Checkbox(!on),
                    }],
                    Some(window),
                    cx,
                );
            }
            PropertyAction::Remove { key } => {
                self.property_field = None;
                self.apply_property_changes(&[props::Change::Remove { key }], Some(window), cx);
            }
            PropertyAction::Retype { key, ty } => {
                self.property_menu = None;
                if let Some(row) = self.property_rows().into_iter().find(|r| r.key == key) {
                    let change = properties::retype(&row, ty);
                    self.apply_property_changes(&[change], Some(window), cx);
                }
                // The assignment belongs in the vault's `types.json`, which
                // the workspace owns; it writes the file and hands the new
                // table back through `set_property_types`.
                self.property_types
                    .types
                    .insert(key.clone(), ty.as_str().to_string());
                cx.emit(EditorEvent::PropertyTypeAssigned { key, ty });
                cx.notify();
            }
            PropertyAction::AddRow => {
                self.commit_property_field(Some(window), cx);
                self.property_field = Some(Field::new(Target::New, Part::Key, ""));
                cx.notify();
            }
            PropertyAction::OpenTypeMenu { key } => {
                // A second click on the same icon closes the menu.
                self.property_menu = if self.property_menu.as_deref() == Some(key.as_str()) {
                    None
                } else {
                    Some(key)
                };
                cx.notify();
            }
        }
    }

    /// Writes the open field into the document and closes it. Returns whether
    /// a field was open.
    /// The field the keys go to while one is open: a property row's, or a
    /// table cell's.
    ///
    /// Taking the field drops a selected pill: the selection is a step in
    /// the two-press Backspace gesture and survives nothing else, so typing
    /// or moving the caret puts focus back in the input. The gates that only
    /// ask whether a field is open go through [`Editor::field_kind`] instead,
    /// to leave the selection alone.
    fn active_field(&mut self) -> Option<&mut Field> {
        self.selected_pill = None;
        match self.property_field.as_mut() {
            Some(f) => Some(f),
            None => self.table_field.as_mut().map(|c| &mut c.field),
        }
    }

    /// A click on a table cell: commit whatever field was open, then take
    /// this cell's text into a new one.
    fn table_action(&mut self, action: TableAction, window: &mut Window, cx: &mut Context<Self>) {
        let TableAction::Edit { span } = action;
        self.commit_property_field(Some(window), cx);
        // The click arrives in the coordinates the widget was built from,
        // which the commit may shift.
        let span = self
            .commit_table_field(Some(window), cx, Some(span.clone()))
            .unwrap_or(span);
        self.open_cell(span, cx);
    }

    /// Takes the cell's text into a field for typing.
    fn open_cell(&mut self, span: Range<usize>, cx: &mut Context<Self>) {
        let Some(was) = self.buf.text().get(span.clone()).map(str::to_string) else {
            return;
        };
        self.table_field = Some(CellEdit {
            span,
            field: Field::new(Target::New, Part::Value, was.clone()),
            was,
        });
        cx.notify();
    }

    /// Writes the open cell back into the document, if it changed.
    ///
    /// Obsidian re-pads a table only when a table command runs, so a cell
    /// edit replaces that cell's bytes and leaves the rest of the table
    /// exactly as it was. Returns `keep` shifted past the edit, so the cell
    /// a click asked for is still the right bytes afterwards.
    fn commit_table_field(
        &mut self,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
        keep: Option<Range<usize>>,
    ) -> Option<Range<usize>> {
        let CellEdit { span, was, field } = self.table_field.take()?;
        let text = table::cell_text(&field.text);
        // The bytes have to be the ones the field was opened on: anything
        // else means the document moved underneath, and the vault is the
        // source of truth.
        if self.buf.text().get(span.clone()) != Some(was.as_str()) || text == was {
            cx.notify();
            return keep;
        }
        let grew = text.len() as isize - was.len() as isize;
        self.edit(span.clone(), &text, EditKind::Other, window, cx);
        cx.notify();
        keep.map(|k| {
            if k.start >= span.end {
                (k.start as isize + grew) as usize..(k.end as isize + grew) as usize
            } else {
                k
            }
        })
    }

    /// Drops the open cell without writing it.
    fn cancel_table_field(&mut self, cx: &mut Context<Self>) -> bool {
        let had = self.table_field.take().is_some();
        if had {
            cx.notify();
        }
        had
    }

    /// Opens the next cell in reading order (`dx`) or the cell below
    /// (`dy`), committing the one that is open. `false` when there is no
    /// such cell, so Tab out of the last one falls through to the editor.
    fn step_table_cell(
        &mut self,
        dx: isize,
        dy: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(span) = self.table_field.as_ref().map(|c| c.span.clone()) else {
            return false;
        };
        self.commit_table_field(Some(window), cx, None);
        // The table is re-read after the commit: the cell's own bytes may
        // have changed length.
        let Some(at) = table::table_at(self.buf.text(), span.start) else {
            return false;
        };
        let Some((row, col)) = at.cells.iter().enumerate().find_map(|(r, cells)| {
            cells
                .iter()
                .position(|s| s.start <= span.start && span.start <= s.end)
                .map(|c| (r, c))
        }) else {
            return false;
        };
        let next = if dy != 0 {
            row.checked_add_signed(dy)
                .and_then(|row| at.cells.get(row))
                .and_then(|cells| cells.get(col))
                .cloned()
        } else if dx > 0 {
            // Past the last cell of a row, Tab goes on to the next row.
            match at.cells.get(row).map(|c| c.len()) {
                Some(len) if col + 1 < len => at.cells[row].get(col + 1).cloned(),
                _ => at.cells.get(row + 1).and_then(|c| c.first()).cloned(),
            }
        } else {
            match col.checked_sub(1) {
                Some(c) => at.cells[row].get(c).cloned(),
                None => row
                    .checked_sub(1)
                    .and_then(|r| at.cells.get(r))
                    .and_then(|c| c.last())
                    .cloned(),
            }
        };
        let Some(next) = next else {
            return false;
        };
        self.open_cell(next, cx);
        self.table_field.is_some()
    }

    /// Enter in a properties field. It commits, and in the trailing input of
    /// a list value Obsidian keeps that input focused so the next item can be
    /// typed straight away — "Enter in the trailing input adds an item", with
    /// the caret still there afterwards. The bytes are the commit's alone: a
    /// freshly opened input writes nothing until it is typed into.
    fn enter_property_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let again = match self.property_field.as_ref() {
            Some(field) => match (field.key(), field.list.as_ref()) {
                (Some(key), Some(list)) if list.index >= list.items.len() => Some(key.to_string()),
                _ => None,
            },
            None => None,
        };
        self.commit_property_field(Some(window), cx);
        // The row is read after the commit: its list has just grown by the
        // item that was typed, and the new input appends after it.
        if let Some(row) =
            again.and_then(|key| self.property_rows().into_iter().find(|row| row.key == key))
        {
            self.property_field = Some(row.append_field());
        }
        cx.notify();
    }

    /// Backspace in a properties field: Obsidian's two-press pill gesture in
    /// the trailing input of a list value, and a character delete anywhere
    /// else. The first press moves focus to the last pill and writes
    /// nothing, the second removes it through the same change the pill's own
    /// remove control makes, and repeating walks backwards a pill at a time;
    /// with the list empty the input has focus again and the press does
    /// nothing (see [`properties::PillStep`]).
    fn backspace_property_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self.selected_pill;
        let Some(field) = self.property_field.as_mut() else {
            return;
        };
        let key = field.key().map(str::to_string);
        let step = field.backspace(selected);
        self.selected_pill = match step {
            PillStep::Select(ix) => Some(ix),
            _ => None,
        };
        if let (PillStep::Remove(index), Some(key)) = (step, key) {
            let change = self
                .property_rows()
                .into_iter()
                .find(|row| row.key == key)
                .and_then(|row| properties::remove_item(&row, index));
            if let Some(change) = change {
                self.apply_property_changes(&[change], Some(window), cx);
            }
            // The field carries the list as it was before the removal, so it
            // is rebuilt from the row as it now stands: committing the stale
            // snapshot would write the removed item back.
            self.property_field = self
                .property_rows()
                .into_iter()
                .find(|row| row.key == key)
                .map(|row| row.append_field());
        }
        cx.notify();
    }

    fn commit_property_field(
        &mut self,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) -> bool {
        self.selected_pill = None;
        let Some(field) = self.property_field.take() else {
            return false;
        };
        let ty = match field.key() {
            Some(key) => self.property_type_of(key),
            None => props::PropertyType::Text,
        };
        if let Some(change) = properties::commit(&field, ty) {
            self.apply_property_changes(&[change], window, cx);
        }
        cx.notify();
        true
    }

    /// Drops the open field without writing it.
    fn cancel_property_field(&mut self, cx: &mut Context<Self>) -> bool {
        self.selected_pill = None;
        let had = self.property_field.take().is_some();
        if had {
            cx.notify();
        }
        had
    }

    /// A click on a Live Preview widget: an embed and a table keep it
    /// (their links and cells take their own clicks), and a math block
    /// opens for editing at the clicked row. Returns true when the click
    /// was consumed.
    fn click_widget(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) -> bool {
        let Some(w) = self
            .layout
            .widgets
            .iter()
            .find(|w| w.bounds.contains(&position))
            .cloned()
        else {
            return false;
        };
        if w.embed {
            return true;
        }
        let lh = self.layout.line_height;
        let rows = w.lines.len().max(1);
        let row_h = (w.bounds.size.height / rows as f32).max(lh);
        let row = ((position.y - w.bounds.origin.y) / row_h).floor().max(0.) as usize;
        let line = (w.lines.start + row)
            .min(w.lines.end - 1)
            .min(self.line_count() - 1);
        self.buf.collapse_cursors();
        self.buf.head = self.buf.line_starts()[line];
        self.buf.anchor = self.buf.head;
        self.buf.goal = None;
        self.buf.marked = None;
        cx.notify();
        true
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
        if self.scrollbar_mouse_down(event.position, window, cx) {
            return;
        }
        if self.click_widget(event.position, cx) {
            return;
        }
        if self.click_rendered(event, window, cx) {
            return;
        }
        let Some(at) = self.offset_at_point(window, event.position) else {
            return;
        };
        self.buf.goal = None;
        self.buf.marked = None;
        if event.modifiers.alt && event.click_count == 1 {
            // Alt+click adds a cursor (or removes the one it lands on).
            let mut cursors = self.buf.cursors();
            if let Some(ix) = cursors
                .sels
                .iter()
                .position(|s| s.is_empty() && s.head == at)
            {
                if cursors.sels.len() > 1 {
                    cursors.sels.remove(ix);
                    cursors.primary = cursors.primary.min(cursors.sels.len() - 1);
                }
            } else {
                cursors.sels.push(Sel::at(at));
                cursors.primary = cursors.sels.len() - 1;
            }
            self.buf.set_cursors(cursors);
            cx.notify();
            return;
        }
        if !event.modifiers.shift {
            self.buf.collapse_cursors();
        }
        match event.click_count {
            2 => {
                let word = self.buf.word_at(at);
                self.buf.anchor = word.start;
                self.buf.head = word.end;
            }
            n if n >= 3 => {
                let line = self.buf.cursor_at(at).line;
                self.buf.anchor = self.buf.line_starts()[line];
                self.buf.head = self
                    .buf
                    .line_starts()
                    .get(line + 1)
                    .copied()
                    .unwrap_or(self.buf.len());
            }
            _ => {
                self.buf.head = at;
                if !event.modifiers.shift {
                    self.buf.anchor = at;
                }
                self.dragging = true;
            }
        }
        cx.notify();
    }

    fn mouse_drag(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        if self.thumb_drag.is_some() {
            self.scrollbar_drag(position, window, cx);
            return;
        }
        if !self.dragging {
            return;
        }
        let bounds = self.layout.bounds;
        let lh = self.layout.line_height;
        if position.y < bounds.origin.y {
            self.scroll_by(-lh, window, cx);
        } else if position.y > bounds.bottom_left().y {
            self.scroll_by(lh, window, cx);
        }
        let clamped = point(
            position.x.max(bounds.origin.x).min(bounds.bottom_right().x),
            position
                .y
                .max(bounds.origin.y)
                .min(bounds.bottom_right().y - px(1.)),
        );
        if let Some(at) = self.offset_at_point(window, clamped) {
            self.buf.head = at;
            cx.notify();
        }
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
                // The same normalization the reading model does: Obsidian
                // renders and copies with `\n` endings whatever the file
                // holds, and drops exactly one trailing newline.
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
        // A properties field takes the keystrokes while it is open; the
        // document changes once, when the field commits.
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
            // A ranged replacement (IME) addresses the primary cursor's line.
            Some(range) => self.edit(range, new_text, kind, Some(window), cx),
            // Auto-pairing is a typed character only: a paste has its own
            // path and an IME commit arrives as a range above.
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
        // The popup is drawn over the text, anchored under the query it is
        // offering rows for.
        let popup = self.suggest.take().map(|state| {
            let element = self
                .suggest_anchor(&state)
                .map(|anchor| crate::suggest::render(&state, &anchor, &theme, cx));
            self.suggest = Some(state);
            element
        });
        let editor = div()
            // A second identifier while the popup is up: its own bindings
            // match deeper than the workspace's and so shadow them.
            .key_context(if self.suggest.is_some() {
                "Editor SuggestPopup"
            } else {
                "Editor"
            })
            .track_focus(&self.focus_handle)
            .size_full()
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
        div()
            .relative()
            .size_full()
            .on_drop(cx.listener(Self::handle_drop))
            .child(editor)
            .children(panel)
            .children(popup.flatten())
            .children(preview)
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
        // A toggle shows its state with the accent color.
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
}

// ---------------------------------------------------------------------------
// Element

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
                // The setext underline row is the heading's markup: it keeps
                // the heading colour and its own line, but body weight and
                // body size (`line_block`), which is what Obsidian computes
                // for `cm-formatting-header` — 16 px at 400.
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
    // The stretches no construct covers take the same treatment, which on a
    // done task is most of the line.
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
                source, len, kind, ..
            } => {
                if *len == 0 {
                    continue;
                }
                let kinds = flat
                    .iter()
                    .find(|(r, _)| r.start <= source.start && source.start < r.end)
                    .map(|(_, k)| k.clone())
                    .unwrap_or_else(|| vec![*kind]);
                push(run_for(&kinds, base, *len, p, done));
            }
        }
    }
    if runs.is_empty() {
        runs.push(base.to_run(0));
    }
    runs
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
    // Headings keep Obsidian's tighter line height, never below the body's.
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
    // A line with a tall inline formula (a fraction, a sum with limits)
    // grows to hold it, the way CodeMirror's line grows around an inline
    // widget; every wrapped row of the line shares the height.
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
        // The listeners this element registers are raw window listeners,
        // which fire for any point inside its bounds; a panel over the text
        // — the find panel, a modal — owns its own clicks, and this hitbox
        // is how a listener asks whether one of those took the point first.
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let text_style = window.text_style();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let lh = window.line_height();
        let theme = cx.global::<Theme>().clone();
        let entity = self.editor.clone();
        // The editor is leased for the frame so widgets (GPUI elements
        // built from the reading renderer) can be laid out with the app
        // at hand while its lines are read.
        self.editor.update(cx, |editor, cx| {
            // Measured before any line is shaped, so this frame's inline
            // formulas already have placeholders.
            editor.spaces = Some((
                font_size,
                SpaceWidths::measure(window.text_system(), &text_style.font(), font_size),
            ));
            // A cursor sent into a fold from outside (a link, a search hit,
            // an outline row) opens it before anything is laid out.
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
            let content_x = bounds.origin.x + (bounds.size.width - content_width) / 2.;
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
            // The properties block's fold state travels in the toggled set,
            // the same place the reading view keeps callout folds.
            let mut no_toggles = editor.callout_folds.clone();
            if editor.properties_folded {
                no_toggles.insert(crate::properties::PROPERTIES_FOLD_ID);
            }
            let property_types = editor.property_types.clone();
            // Live Preview's properties block is editable: the open field is
            // drawn with its caret, and clicks come back as actions.
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
                    // Inside a fold: these lines are not on screen at all.
                    line = end + 1;
                    continue;
                }
                if let Some((wlines, widget)) = editor.folded_widget(line) {
                    let wlines = wlines.clone();
                    if line != wlines.start || editor.widget_is_hidden(widget) {
                        // Scrolled into a folded widget: its lines take no
                        // room. A hidden properties block takes none at all.
                        line = wlines.end;
                        continue;
                    }
                    // An embed keeps its clicks so its links work, and a
                    // table keeps them because its cells take their own:
                    // neither ever unfolds to its source.
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
                            // The editor's own window listener runs ahead
                            // of this element and would put the caret on
                            // the fence line; Obsidian's flair leaves the
                            // caret where it was, so the mouse stops here.
                            .block_mouse_except_scroll()
                            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                // An empty fence copies nothing at all, the
                                // same rule the reading view's button
                                // follows -- and with nothing copied there
                                // is nothing to announce.
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
                        // Obsidian's rule is `right: 6px; top: 6px` against
                        // a line box with padding of its own, which comes
                        // out very nearly centred on the fence line: its
                        // flair is 26 px tall on a 29 px line. Centring it
                        // here is that picture without the padding.
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
                // The fold indicator: Obsidian's `cm-fold-indicator`, a
                // hover-revealed chevron at `left: -15px` on a heading line
                // and `-10px` on a list line, shown whenever the line is
                // folded. Mouse-down rather than click, blocking the mouse,
                // for the reason the callout chevron gives.
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
                    // Matches touching this line, by binary search on their starts.
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
                                selection_quads.push(fill(
                                    Bounds::new(
                                        point(content_x + x0, y + rh * r as f32),
                                        size(x1 - x0, rh),
                                    ),
                                    palette.selection,
                                ));
                            }
                        }
                    }
                    if sel.head >= range.start && sel.head <= range.end {
                        let (row, x) = shaped.row_and_x(sel.head - range.start);
                        cursor_quads.push(fill(
                            Bounds::new(
                                point(content_x + x, y + rh * row as f32),
                                size(px(2.), rh),
                            ),
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
                // Whatever the reveal that got us here was computed from
                // has to be what the measurement is checked against, or the
                // first jump over a widget never gets its correction: a
                // widget nothing had measured yet was estimated by
                // `height_of` at one line each, and a table or a code fence
                // is nothing like that tall. A widget that grew later -- an
                // image that has just loaded -- is the same case with a real
                // previous height.
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
                    // The input handler needs the cursor line's layout between
                    // frames; shaping it again hits GPUI's per-frame layout cache.
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
            // The part of the first line scrolled off the top moves the
            // thumb between lines, so it does not step.
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
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            for quad in prepaint.decor.drain(..) {
                window.paint_quad(quad);
            }
            for quad in prepaint.selection.drain(..) {
                window.paint_quad(quad);
            }
            for (visible, shaped) in prepaint.lines.iter() {
                let origin = point(content_x, bounds.origin.y + visible.top);
                // Run backgrounds (code, highlights) are a separate pass.
                let _ = shaped.line.paint_background(
                    origin,
                    visible.row_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
                let _ = shaped.line.paint(
                    origin,
                    visible.row_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
                // Inline formulas over their placeholders, on the text
                // baseline and centred in the placeholder's slack, in the
                // text colour: rendered math is not markup.
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
            }
            for w in prepaint.widgets.iter_mut() {
                w.element.paint(window, cx);
            }
            // Over the lines: a flair sits on its fence line's text.
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
            // `should_handle_scroll` rather than `is_hovered`: a wheel event
            // is not an interaction with what is directly under the pointer.
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
                // A notify during paint is dropped; ask for the next frame.
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
        // The six display names that were read out of a running Obsidian,
        // an unregistered info string kept verbatim, and a fence with none
        // at all, whose flair is there with an empty label.
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
        // A note of eight blocks whose only indented one was the fifth
        // showed seven flairs, so the fence line is what carries it.
        assert_eq!(flairs("    four space indented\n    second line\n"), vec![]);
        assert_eq!(
            flairs("> ```\n> in a blockquote\n> ```\n"),
            vec![(0, String::new(), "in a blockquote".into())]
        );
        assert_eq!(
            flairs("- item\n\n  ```js\n  in a list\n  ```\n"),
            vec![(2, "JavaScript".into(), "in a list".into())]
        );
        // A CRLF file's block copies as LF, and exactly one trailing
        // newline comes off -- the rule the reading view's button follows.
        assert_eq!(
            flairs("```\r\none\r\ntwo\r\n\r\n```\r\n"),
            vec![(0, String::new(), "one\ntwo\n".into())]
        );
        // An empty fence carries a flair that copies nothing; the click
        // path writes nothing rather than emptying the clipboard.
        assert_eq!(
            flairs("```\n```\n"),
            vec![(0, String::new(), String::new())]
        );
    }

    #[test]
    fn end_of_display_is_the_end_of_the_source_line() {
        // Trailing markup displays nothing, so `to_source` has no piece to
        // match at the end and walks back into the link text; the click
        // path must land after the `]]` instead.
        let d = display("see [[Welcome]]");
        assert_eq!(d.text, "see Welcome");
        assert_eq!(d.to_source(d.text.len()), 13);
        assert_eq!(source_index(&d, d.text.len()), 15);

        let d = display("ab `cd`");
        assert_eq!(d.text, "ab cd");
        assert_eq!(source_index(&d, d.text.len()), 7);

        let d = display("- [ ] task");
        assert_eq!(d.text, "☐ task");
        assert_eq!(source_index(&d, d.text.len()), 10);
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

        // A string that is not one character is not a keystroke: it goes
        // in as it stands, the way a paste does.
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
        // The last glyph of the link text is still inside the link's span,
        // which is what makes a click on it follow the link.
        assert!(source_index(&d, 10) < 15);

        let d = display("plain line");
        assert_eq!(source_index(&d, 4), 4);
        assert_eq!(source_index(&d, d.text.len()), 10);
    }
}
