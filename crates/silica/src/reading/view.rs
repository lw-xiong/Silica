//! GPUI rendering of a reading-view [`Model`].
//!
//! Top-level blocks go through a lazily measured `list`, so a long note only
//! lays out the blocks on screen. Inline content is one `InteractiveText`
//! per run of text, with the styled ranges from the model and clickable
//! ranges for links, tags and footnote references.

use super::folds::ReadingFoldStore;
use super::model::{
    Embed, EmbedKind, HtmlBlockKind, InlineStyle, LinkTarget, Model, RBlock, RItem, RichText,
    Segment, TaskMarker,
};
use crate::assets::{callout_icon, icon};
use crate::fold::{self, FoldKey};
use crate::math::{self, FormulaElement, Inline, Overlay, SpaceWidths};
use crate::properties::{Field, Part, Target};
use crate::properties::{PROPERTIES_FOLD_ID, PROPERTY_ID_BASE};
use crate::theme::Theme;
use gpui::{
    actions, canvas, div, img, list, prelude::*, px, AnyElement, AnyView, App, ClipboardItem,
    Context, ElementId, EventEmitter, FocusHandle, Focusable, Font, FontStyle, FontWeight,
    HighlightStyle, Hsla, InteractiveElement, InteractiveText, KeyBinding, ListAlignment,
    ListOffset, ListState, MouseButton, ObjectFit, Pixels, SharedString, StrikethroughStyle,
    StyledText, Task, TextRun, UnderlineStyle, WeakEntity, Window, WindowTextSystem,
};
use rodio::{OutputStream, Sink};
use serde_json::{json, Value};
use silica_formats::config::PropertyTypes;
use silica_formats::properties::{Change, PropertiesInDocument, Property, PropertyType};
use silica_formats::FileKind;
use silica_markdown::Alignment;
use silica_plugins::live;
use std::cell::RefCell;
use std::collections::HashSet;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

mod render;

pub(crate) use render::Renderer;

actions!(
    reading,
    [
        ScrollPageDown,
        ScrollPageUp,
        ScrollLineDown,
        ScrollLineUp,
        ScrollToTop,
        ScrollToBottom,
        Copy,
        SelectAll,
    ]
);

/// Binds the reading view's scrolling keys in its own context; call once at
/// startup. Obsidian's reading view is a DOM scroller with no commands of its
/// own, so this is the browser's set of page-scrolling keys rather than a
/// probed Obsidian binding: the page keys and Space for a page, the arrows
/// for a line, Home and End (plain and with Ctrl, which a document scroller
/// treats alike) for the ends.
pub fn bind_keys(cx: &mut App) {
    let ctx = Some("ReadingView");
    cx.bind_keys([
        KeyBinding::new("pagedown", ScrollPageDown, ctx),
        KeyBinding::new("pageup", ScrollPageUp, ctx),
        KeyBinding::new("space", ScrollPageDown, ctx),
        KeyBinding::new("shift-space", ScrollPageUp, ctx),
        KeyBinding::new("down", ScrollLineDown, ctx),
        KeyBinding::new("up", ScrollLineUp, ctx),
        KeyBinding::new("home", ScrollToTop, ctx),
        KeyBinding::new("ctrl-home", ScrollToTop, ctx),
        KeyBinding::new("end", ScrollToBottom, ctx),
        KeyBinding::new("ctrl-end", ScrollToBottom, ctx),
        KeyBinding::new("ctrl-c", Copy, ctx),
        KeyBinding::new("ctrl-a", SelectAll, ctx),
    ]);
}

/// What `render` multiplies the base font size by for the line height, and so
/// how tall the line an arrow key scrolls is.
const LINE_HEIGHT: f32 = 1.6;

/// Pixels a page key scrolls a viewport this tall: the viewport less an eighth
/// of it capped at 40 px, which is what a browser's page keys do, so a strip of
/// the page just read stays on screen. A viewport of zero is a view that has
/// not been painted yet, which has nothing to scroll.
pub(crate) fn page_distance(viewport: f32) -> f32 {
    if viewport <= 0. {
        return 0.;
    }
    viewport - (viewport / 8.).min(40.)
}

/// Pixels an arrow key scrolls: one rendered line.
pub(crate) fn line_distance(base_font_size: f32) -> f32 {
    base_font_size * LINE_HEIGHT
}

/// Whether a [`ReadingView::set_note`] is a reload of the note already on
/// screen, which keeps the reader's place and their folds. Moving to another
/// note, or to no note at all, drops both.
pub(crate) fn same_note(current: Option<&str>, next: Option<&str>) -> bool {
    next.is_some() && current == next
}

/// Something the reader did to a property row. The reading view raises none
/// of these; Live Preview turns each into a document edit.
#[derive(Debug, Clone, PartialEq)]
pub enum PropertyAction {
    /// Start typing in a row's key or value.
    Edit {
        target: Target,
        part: Part,
    },
    /// Start typing in one item of a list-shaped value. `index` past the
    /// last item is the empty input Obsidian keeps after the pills, where
    /// a new item is written.
    EditItem {
        key: String,
        index: usize,
    },
    /// A pill's own remove control: drop that item and leave the rest.
    RemoveItem {
        key: String,
        index: usize,
    },
    Remove {
        key: String,
    },
    Retype {
        key: String,
        ty: PropertyType,
    },
    /// Open an unnamed row at the end of the block.
    AddRow,
    /// Show the type menu for a row.
    OpenTypeMenu {
        key: String,
    },
}

/// Lets the properties block be edited. The reading view passes `None` and
/// draws the rows read-only; Live Preview passes the open field and a sink
/// for what the reader does.
pub(crate) struct PropertiesHost {
    /// The field being typed into, when it belongs to this block.
    pub field: Option<Field>,
    /// The pill a Backspace selected, by item index in `field`'s row: the
    /// first of the two presses that remove it moves focus there, and the
    /// pill is drawn focused until the second press takes it.
    pub selected_pill: Option<usize>,
    /// `propertiesInDocument`: `Hidden` draws nothing at all.
    pub mode: PropertiesInDocument,
    /// The key whose type menu is open, if any.
    pub menu: Option<String>,
    pub on_action: PropertyHandler,
}

/// What a property row's affordances do when clicked.
pub type PropertyHandler = Rc<dyn Fn(PropertyAction, &mut Window, &mut App)>;

/// Something the reader did to a table cell. The reading view raises none
/// of these; Live Preview turns each into a document edit.
#[derive(Debug, Clone, PartialEq)]
pub enum TableAction {
    /// Start typing in the cell standing for these bytes.
    Edit { span: Range<usize> },
}

/// What a table cell does when clicked.
pub type TableHandler = Rc<dyn Fn(TableAction, &mut Window, &mut App)>;

/// Lets a table's cells be edited. The reading view passes `None` and draws
/// the cells read-only; Live Preview passes the open cell and a sink for
/// what the reader does.
pub(crate) struct TableHost {
    /// The cell being typed into: the bytes it stands for and the text in
    /// hand, which has not reached the document yet.
    pub cell: Option<(Range<usize>, Field)>,
    pub on_action: TableHandler,
}

/// What a rendered link does when clicked.
pub type LinkHandler = Rc<dyn Fn(LinkTarget, &mut Window, &mut App)>;
/// What a foldable callout's title does when clicked, by title id.
pub type ToggleHandler = Rc<dyn Fn(usize, &mut App)>;
/// What a task checkbox does when clicked.
pub type TaskHandler = Rc<dyn Fn(TaskMarker, &mut App)>;
/// Told which code block's copy button was pressed, so its check can show.
pub type CopyHandler = Rc<dyn Fn(usize, &mut App)>;
/// What resolves a hovered link into a page preview view.
pub type PreviewHandler = Rc<dyn Fn(&LinkTarget, &mut Window, &mut App) -> Option<AnyView>>;

/// The settings and affordances only the reading view has. Live Preview
/// passes `None` and keeps the click handling it already does on the
/// source line behind a widget.
pub(crate) struct ReadingHost {
    /// `propertiesInDocument`: `Hidden` draws no properties block.
    pub properties: PropertiesInDocument,
    /// The code block whose copy button is showing its check.
    pub copied: Option<usize>,
    pub on_copy: CopyHandler,
    pub on_task: TaskHandler,
    pub preview_resolver: Option<PreviewHandler>,
    /// Play (or pause, or seek) a local video in the app's own player.
    pub on_video: VideoHandler,
    /// What the player is showing, if anything: the embed draws its frame
    /// and its clock from it.
    pub video: Rc<RefCell<Option<VideoState>>>,
    /// An embedded `.canvas`, which needs a view of its own: given the
    /// vault path and the size the note leaves it, answer an element.
    pub on_canvas: CanvasHandler,
}

/// An `![[Board.canvas]]`: the vault path and the size the note gives it.
///
/// No window: the answer is a canvas view the workspace already made, and
/// putting an entity in the tree is not something a paint has to ask for.
pub type CanvasHandler = Rc<dyn Fn(&str, f32, f32) -> AnyElement>;

/// A canvas embed with no host behind it: a card naming the file, which the
/// reading view's link handler opens on a click.
pub(crate) fn missing_canvas(path: &str, _w: f32, _h: f32) -> AnyElement {
    let label = path.rsplit('/').next().unwrap_or(path).to_string();
    div()
        .id(SharedString::from(format!("canvas-embed-{path}")))
        .px_3()
        .py_2()
        .rounded_md()
        .border_1()
        .border_color(gpui::Hsla {
            h: 0.0,
            s: 0.0,
            l: 0.0,
            a: 0.12,
        })
        .cursor_pointer()
        .child(SharedString::from(label))
        .into_any_element()
}

/// A local image the vault holds no file for: a card naming it, the way
/// every other missing attachment reads. gpui paints nothing at all for a
/// source it cannot read — no image, no error — which leaves a blank the
/// reader cannot explain, so the fallback names the file instead.
pub(crate) fn missing_image(path: &std::path::Path) -> AnyElement {
    let label = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    div()
        .id(SharedString::from(format!("image-embed-{label}")))
        .my_1()
        .px_3()
        .py_2()
        .rounded_md()
        .border_1()
        .border_color(gpui::Hsla {
            h: 0.0,
            s: 0.0,
            l: 0.0,
            a: 0.12,
        })
        .cursor_pointer()
        .child(SharedString::from(label))
        .into_any_element()
}

/// A click on a video embed: the file, and `Some(share)` when the click was
/// on the position bar (a seek) rather than the picture or the button.
pub type VideoHandler = Rc<dyn Fn(&std::path::Path, Option<f32>, &mut Window, &mut App)>;

pub enum ReadingEvent {
    OpenLink(LinkTarget),
    /// A Ctrl+click on a link: it opens in a new tab.
    OpenLinkInNewTab(LinkTarget),
    /// The reader changed a property in the reading view's properties block.
    /// The workspace turns this into the same edit Live Preview makes.
    PropertyChanged {
        path: String,
        changes: Vec<silica_formats::properties::Change>,
    },
    /// The reader committed an inline title edit: rename the file to the new
    /// stem. The workspace renames the file and rewrites its links.
    RenameFile {
        path: String,
        new_name: String,
    },
}

/// A reader clicked a task checkbox. The reading view does not write the
/// note itself: the workspace turns this into the same edit Live Preview
/// makes when the same box is clicked there.
pub struct TaskToggled {
    /// The note the reading view was showing, so an event that outlived
    /// its note writes nothing.
    pub path: String,
    /// Where the three bytes `[`, status, `]` are in it.
    pub marker: TaskMarker,
}

pub struct ReadingView {
    focus_handle: FocusHandle,
    pub path: Option<String>,
    model: Arc<Model>,
    list: ListState,
    /// The fold store of this vault's notes, opened once at startup. The
    /// record lives outside the vault, keyed by vault-relative path.
    fold_store: ReadingFoldStore,
    /// Callouts whose fold state the reader flipped, by title id.
    toggled: HashSet<usize>,
    /// The note's heading and list folds, by key — the same keys the
    /// editor keeps, handed across on a mode switch. Only the heading ones
    /// collapse anything here; the list ones ride along so a switch to the
    /// editor and back loses nothing.
    folds: Vec<FoldKey>,
    /// `foldHeading`: with it off the indicator is not drawn.
    fold_headings: bool,
    /// Per top-level block: inside a collapsed heading's section.
    hidden: Vec<bool>,
    readable_line_length: bool,
    /// `showInlineTitle`: the file's name above the note, as an editable
    /// title. It sits above the scroller rather than scrolling with the
    /// text, which is the one visible difference from Obsidian's.
    inline_title: bool,
    /// The inline title is being edited: the text the user has typed. `None`
    /// when the title is just a static label.
    inline_title_edit: Option<String>,
    properties_in_document: PropertiesInDocument,
    /// The vault's `types.json`, for the type each property row shows.
    property_types: PropertyTypes,
    /// The code block whose copy button is showing its check, and the
    /// timer that takes it away again.
    copied: Option<usize>,
    flash: Option<Task<()>>,
    /// Width of the view as last painted; tables lay their columns out for it.
    width: Pixels,
    pub preview_resolver: Option<PreviewHandler>,
    /// Where an `![[x.canvas]]` embed gets its element. The workspace sets
    /// this because it owns the canvas views; without it the embed falls
    /// back to a card that opens the file.
    pub canvas_host: Option<CanvasHandler>,
    /// The in-app video player: what it is showing, where, and the frame.
    ///
    /// Behind a cell because the embeds paint through `&self` and the player
    /// is driven from a click inside them; the read loop is a task that
    /// holds the decoder, so the entry disappearing is what stops `ffmpeg`.
    pub video: Rc<RefCell<Option<VideoState>>>,
    /// The property field the reader is typing into, if any: its key or
    /// value, or one item of a list-shaped value. The field's text lives
    /// here while the caret is in it, and a commit writes it back to the
    /// vault through `ReadingEvent::PropertyChanged`.
    pub property_field: Option<Field>,
    /// The pill a Backspace selected in an open list field: the first of
    /// the two presses that remove it, drawn focused until the second takes
    /// it.
    pub selected_pill: Option<usize>,
    /// The key whose type menu is open, if any.
    pub property_menu: Option<String>,
    /// The property entries of the last-rendered properties block, so a
    /// pill remove or retype can look up the row it acts on. Set from
    /// `render_block` whenever a properties block is painted.
    pub property_entries: Option<Vec<silica_formats::properties::Property>>,
    /// The reader's text selection: one entry per selected RichText run,
    /// holding the block index, the run's id, and the byte range within it.
    /// `None` when nothing is selected.
    pub selection: Option<Vec<(usize, usize, usize, usize)>>,
    /// The block index where the current drag selection started, and the
    /// byte offset within its first RichText run. `None` when no drag is in
    /// progress.
    pub drag_start: Option<(usize, usize)>,
    /// `backlink:toggle-backlinks-in-document`: whether the note's bottom
    /// shows its Backlinks section in reading view.
    pub backlinks_in_document: bool,
    /// The rows of that section: the linking note and the snippet around
    /// the link. The workspace fills them from the cache when a note opens.
    pub document_backlinks: Vec<DocBacklink>,
}

/// One row of the reading view's in-document Backlinks section.
#[derive(Clone)]
pub struct DocBacklink {
    /// Vault-relative path of the note the link was written in.
    pub source: String,
    /// The line around the link, trimmed.
    pub context: String,
}

/// One video in the player.
pub struct VideoState {
    /// The absolute path being played.
    pub path: PathBuf,
    /// Size, frame rate and length, from `ffprobe`.
    pub info: crate::video::Info,
    pub playing: bool,
    /// Where the decoder is, in seconds.
    pub at: f32,
    /// The frame on screen.
    pub frame: Option<Arc<gpui::RenderImage>>,
    /// The read loop; dropping it stops playing and kills `ffmpeg`.
    pub task: Option<Task<()>>,
}

/// A clock reading: `0:07`, `1:03:20`.
fn clock(seconds: f32) -> String {
    let total = seconds.max(0.0) as u64;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

thread_local! {
    /// The audio player for local audio file embeds.
    ///
    /// Thread-local rather than a `static mut`: rodio's stream is not `Send`,
    /// so a `Mutex` cannot hold it, and GPUI runs every event handler on the
    /// main thread anyway. A `static mut` reference is undefined behaviour as
    /// soon as two exist — a fact the compiler cannot know from the comment
    /// that used to stand here.
    static AUDIO_PLAYER: std::cell::RefCell<Option<AudioPlayer>> =
        const { std::cell::RefCell::new(None) };
}

/// An audio player for local audio file embeds. Holds one output stream,
/// one sink (which can play multiple sources sequentially, though we only
/// play one at a time), and play/pause state.
struct AudioPlayer {
    _stream: OutputStream,
    sink: Sink,
    is_playing: bool,
}

impl EventEmitter<ReadingEvent> for ReadingView {}
impl EventEmitter<TaskToggled> for ReadingView {}

impl Focusable for ReadingView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ReadingView {
    pub fn new(cx: &mut Context<Self>) -> ReadingView {
        ReadingView {
            focus_handle: cx.focus_handle(),
            path: None,
            canvas_host: None,
            model: Arc::new(Model::default()),
            list: ListState::new(0, ListAlignment::Top, px(1200.)),
            toggled: HashSet::new(),
            folds: Vec::new(),
            fold_store: ReadingFoldStore::open(),
            fold_headings: true,
            hidden: Vec::new(),
            readable_line_length: true,
            inline_title: true,
            inline_title_edit: None,
            properties_in_document: PropertiesInDocument::default(),
            property_types: PropertyTypes::default(),
            copied: None,
            flash: None,
            width: px(900.),
            preview_resolver: None,
            video: Rc::new(RefCell::new(None)),
            property_field: None,
            selected_pill: None,
            property_menu: None,
            property_entries: None,
            selection: None,
            drag_start: None,
            backlinks_in_document: true,
            document_backlinks: Vec::new(),
        }
    }

    /// What the video player is doing, for the control socket.
    pub fn video_state_json(&self) -> serde_json::Value {
        let video = self.video.borrow();
        match video.as_ref() {
            None => Value::Null,
            Some(v) => json!({
                "path": v.path.display().to_string(),
                "playing": v.playing,
                "at": (v.at * 100.0).round() / 100.0,
                "duration": (v.info.duration * 100.0).round() / 100.0,
                "fps": (v.info.fps * 100.0).round() / 100.0,
                "has_frame": v.frame.is_some(),
            }),
        }
    }

    pub fn set_preview_resolver(&mut self, resolver: Option<PreviewHandler>) {
        self.preview_resolver = resolver;
    }

    pub fn set_readable_line_length(&mut self, on: bool) {
        self.readable_line_length = on;
    }

    /// `backlink:toggle-backlinks-in-document`.
    pub fn set_backlinks_in_document(&mut self, on: bool, cx: &mut Context<Self>) {
        self.backlinks_in_document = on;
        cx.notify();
    }

    /// The rows of the in-document Backlinks section, from the cache.
    pub fn set_document_backlinks(&mut self, rows: Vec<DocBacklink>, cx: &mut Context<Self>) {
        self.document_backlinks = rows;
        cx.notify();
    }

    /// The in-document Backlinks section at the note's bottom: one row per
    /// linking note — its name clickable, the snippet around the link below.
    fn render_document_backlinks(&self, base: f32, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.global::<Theme>().clone();
        let view = cx.entity();
        let on_link: LinkHandler = Rc::new({
            let view = view.clone();
            move |target, window, cx| {
                let modifiers = window.modifiers();
                let new_tab = modifiers.control || modifiers.platform;
                view.update(cx, |_, cx| {
                    cx.emit(if new_tab {
                        ReadingEvent::OpenLinkInNewTab(target)
                    } else {
                        ReadingEvent::OpenLink(target)
                    })
                });
            }
        });
        div()
            .border_t_1()
            .border_color(t.border)
            .mt_4()
            .px_4()
            .pb_4()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_size(px(base * 0.8))
                    .text_color(t.text_muted)
                    .child("Backlinks"),
            )
            .children(self.document_backlinks.iter().map(|b| {
                let source = b.source.clone();
                let label = source
                    .rsplit('/')
                    .next()
                    .unwrap_or(source.as_str())
                    .to_string();
                let on_link = on_link.clone();
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .id(("doc-backlink", source.len()))
                            .text_size(px(base * 0.9))
                            .text_color(t.link)
                            .cursor_pointer()
                            .on_click(move |_, window, cx| {
                                on_link(LinkTarget::Internal(source.clone()), window, cx)
                            })
                            .child(SharedString::from(format!("↗ {label}"))),
                    )
                    .child(
                        div()
                            .text_size(px(base * 0.85))
                            .text_color(t.text_muted)
                            .child(SharedString::from(b.context.clone())),
                    )
                    .into_any_element()
            }))
            .into_any_element()
    }

    pub fn set_inline_title(&mut self, on: bool) {
        self.inline_title = on;
    }

    pub fn set_properties_in_document(&mut self, mode: PropertiesInDocument) {
        self.properties_in_document = mode;
    }

    pub fn set_property_types(&mut self, types: PropertyTypes) {
        self.property_types = types;
    }

    /// Handles a property action from the reading view's properties block.
    /// Writes the change to the vault through `ReadingEvent::PropertyChanged`,
    /// which the workspace turns into the same byte-level edit Live Preview
    /// makes. Field-opening actions (`Edit`, `EditItem`, `AddRow`) only
    /// update local state; the workspace writes the file when the field is
    /// committed.
    fn handle_property_action(&mut self, action: PropertyAction, cx: &mut Context<Self>) {
        // * A click is not a Backspace: whatever pill one selected loses
        //   the selection.
        self.selected_pill = None;
        let Some(path) = self.path.as_deref() else {
            return;
        };
        let Some(entries) = self.property_entries.as_ref() else {
            return;
        };
        let mut next_id = crate::properties::PROPERTY_ID_BASE;
        let rows = crate::properties::rows(
            entries,
            |key| silica_formats::properties::type_of(key, entries, &self.property_types),
            &mut next_id,
        );
        match action {
            PropertyAction::RemoveItem { key, index } => {
                if let Some(row) = rows.iter().find(|r| r.key == key) {
                    if let Some(change) = crate::properties::remove_item(row, index) {
                        cx.emit(ReadingEvent::PropertyChanged {
                            path: path.to_string(),
                            changes: vec![change],
                        });
                    }
                }
            }
            PropertyAction::Retype { key, ty } => {
                if let Some(row) = rows.iter().find(|r| r.key == key) {
                    let change = crate::properties::retype(row, ty);
                    cx.emit(ReadingEvent::PropertyChanged {
                        path: path.to_string(),
                        changes: vec![change],
                    });
                    // * Update the in-memory type table so the row's glyph
                    //   and UI stay consistent with the new type.
                    self.property_types
                        .types
                        .insert(key.clone(), ty.as_str().to_string());
                    self.property_menu = None;
                }
            }
            PropertyAction::AddRow => {
                self.property_field = Some(Field::new(Target::New, Part::Key, ""));
            }
            PropertyAction::Edit { target, part } => {
                self.property_menu = None;
                let row = match (&target, part) {
                    (Target::Existing(key), Part::Value) => {
                        rows.iter().find(|r| &r.key == key).cloned()
                    }
                    _ => None,
                };
                self.property_field = Some(match row {
                    Some(row) => Field::new(
                        Target::Existing(row.key.clone()),
                        Part::Value,
                        row.edit_text.clone(),
                    ),
                    None => Field::new(target, part, String::new()),
                });
            }
            PropertyAction::EditItem { key, index } => {
                self.property_menu = None;
                if let Some(row) = rows.iter().find(|r| r.key == key) {
                    self.property_field = Some(row.item_field(index.min(row.items.len())));
                }
            }
            PropertyAction::OpenTypeMenu { key } => {
                self.property_menu = if self.property_menu.as_deref() == Some(key.as_str()) {
                    None
                } else {
                    Some(key)
                };
            }
            PropertyAction::Remove { key } => {
                cx.emit(ReadingEvent::PropertyChanged {
                    path: path.to_string(),
                    changes: vec![Change::Remove { key }],
                });
            }
        }
    }

    /// Commits the open property field: turns its text into a `Change` and
    /// emits `ReadingEvent::PropertyChanged` so the workspace writes it to
    /// the vault. Clears the field afterwards.
    fn commit_property_field(&mut self, cx: &mut Context<Self>) {
        let Some(field) = self.property_field.take() else {
            return;
        };
        let Some(path) = self.path.as_deref() else {
            return;
        };
        let Some(entries) = self.property_entries.as_ref() else {
            return;
        };
        // * Look up the row's type so `commit` reads the value correctly.
        let ty = field
            .key()
            .and_then(|k| {
                entries.iter().find(|p| p.key == k).map(|p| {
                    silica_formats::properties::type_of(&p.key, entries, &self.property_types)
                })
            })
            .unwrap_or(PropertyType::Text);
        let Some(change) = crate::properties::commit(&field, ty) else {
            return;
        };
        cx.emit(ReadingEvent::PropertyChanged {
            path: path.to_string(),
            changes: vec![change],
        });
    }

    /// Handles key events while a property field is open. Enter commits,
    /// Escape abandons, everything else edits the text. When no field is
    /// open, the event falls through to the view's own bindings (scrolling,
    /// etc.).
    fn field_key_down(
        &mut self,
        _event: &gpui::KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // * This handler only runs when the root div has focus. If a field
        //   is open, every key belongs to it; otherwise the view's scrolling
        //   bindings handle the key via `on_action`.
        if self.property_field.is_none() {
            return;
        }
        cx.stop_propagation();
        let key = _event.keystroke.key.clone();
        match key.as_str() {
            "Enter" => {
                Self::commit_property_field(self, cx);
            }
            "Escape" => {
                self.property_field = None;
            }
            "Backspace" => {
                if let Some(field) = self.property_field.as_mut() {
                    match field.backspace(self.selected_pill) {
                        crate::properties::PillStep::Remove(ix) => {
                            if let Some(key) = field.key() {
                                if let Some(entries) = self.property_entries.as_ref() {
                                    let mut next_id = crate::properties::PROPERTY_ID_BASE;
                                    let rows = crate::properties::rows(
                                        entries,
                                        |k| {
                                            silica_formats::properties::type_of(
                                                k,
                                                entries,
                                                &self.property_types,
                                            )
                                        },
                                        &mut next_id,
                                    );
                                    if let Some(row) = rows.iter().find(|r| r.key == key) {
                                        if let Some(change) =
                                            crate::properties::remove_item(row, ix)
                                        {
                                            cx.emit(ReadingEvent::PropertyChanged {
                                                path: self.path.clone().unwrap_or_default(),
                                                changes: vec![change],
                                            });
                                        }
                                    }
                                }
                            }
                            self.selected_pill = None;
                            self.property_field = None;
                        }
                        crate::properties::PillStep::Select(ix) => {
                            self.selected_pill = Some(ix);
                        }
                        _ => {}
                    }
                }
            }
            "Delete" => {
                if let Some(field) = self.property_field.as_mut() {
                    field.delete();
                }
            }
            "ArrowLeft" => {
                if let Some(field) = self.property_field.as_mut() {
                    field.left();
                }
            }
            "ArrowRight" => {
                if let Some(field) = self.property_field.as_mut() {
                    field.right();
                }
            }
            "Home" => {
                if let Some(field) = self.property_field.as_mut() {
                    field.cursor = 0;
                }
            }
            "End" => {
                if let Some(field) = self.property_field.as_mut() {
                    field.cursor = field.text.len();
                }
            }
            s if s.len() == 1 => {
                if let Some(field) = self.property_field.as_mut() {
                    field.insert(s);
                }
            }
            _ => {}
        }
    }

    /// Copies the current selection to the clipboard. With no selection,
    /// copies the entire note's plain text, as Obsidian's reading view does.
    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let text = if let Some(ranges) = &self.selection {
            self.selection_text(ranges)
        } else {
            self.all_text()
        };
        if !text.is_empty() {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        }
    }

    /// Selects every RichText run in the note.
    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        let mut ranges: Vec<(usize, usize, usize, usize)> = Vec::new();
        for (ix, block) in self.model.blocks.iter().enumerate() {
            self.collect_runs(block, ix, &mut ranges);
        }
        self.selection = if ranges.is_empty() {
            None
        } else {
            Some(ranges)
        };
        self.drag_start = None;
        cx.notify();
    }

    /// Plain text of every RichText run in the note, in document order,
    /// with a newline between blocks.
    fn all_text(&self) -> String {
        let mut out = String::new();
        for block in &self.model.blocks {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&self.block_text(block));
        }
        out
    }

    /// The text of the selected ranges, joined with newlines between
    /// different blocks.
    fn selection_text(&self, ranges: &[(usize, usize, usize, usize)]) -> String {
        let mut out = String::new();
        let mut last_block: Option<usize> = None;
        for (ix, _rich_id, start, end) in ranges {
            let Some(block) = self.model.blocks.get(*ix) else {
                continue;
            };
            if last_block != Some(*ix) {
                if !out.is_empty() {
                    out.push('\n');
                }
                last_block = Some(*ix);
            }
            if let Some(text) = self.rich_text_in_block(block, *start, *end) {
                out.push_str(&text);
            }
        }
        out
    }

    /// Collects every (block_ix, rich_id, 0, len) in `block`'s subtree.
    fn collect_runs(&self, block: &RBlock, ix: usize, out: &mut Vec<(usize, usize, usize, usize)>) {
        match block {
            RBlock::Properties(_) => {}
            RBlock::Paragraph(segments) => {
                for seg in segments {
                    if let Segment::Text(rt) = seg {
                        out.push((ix, rt.id, 0, rt.text.len()));
                    }
                }
            }
            RBlock::Heading { text, .. } => {
                out.push((ix, text.id, 0, text.text.len()));
            }
            RBlock::ThematicBreak => {}
            RBlock::Code { .. } => {}
            RBlock::Html { text: rt, .. } => {
                out.push((ix, rt.id, 0, rt.text.len()));
            }
            RBlock::Quote(children) => {
                for child in children {
                    self.collect_runs(child, ix, out);
                }
            }
            RBlock::Callout {
                title, children, ..
            } => {
                out.push((ix, title.id, 0, title.text.len()));
                for child in children {
                    self.collect_runs(child, ix, out);
                }
            }
            RBlock::List { items, .. } => {
                for item in items {
                    self.collect_item(item, ix, out);
                }
            }
            RBlock::Table { header, rows, .. } => {
                for rt in header {
                    out.push((ix, rt.id, 0, rt.text.len()));
                }
                for row in rows {
                    for rt in row {
                        out.push((ix, rt.id, 0, rt.text.len()));
                    }
                }
            }
            RBlock::Math(_) => {}
            RBlock::Mermaid { .. } => {}
            RBlock::Footnotes(notes) => {
                for (_, blocks) in notes {
                    for block in blocks {
                        self.collect_runs(block, ix, out);
                    }
                }
            }
        }
    }

    /// Collects runs from a list item: its children blocks hold the text.
    fn collect_item(
        &self,
        item: &crate::reading::model::RItem,
        ix: usize,
        out: &mut Vec<(usize, usize, usize, usize)>,
    ) {
        for child in &item.children {
            self.collect_runs(child, ix, out);
        }
    }

    /// Plain text of everything inside `block`, for "copy all".
    fn block_text(&self, block: &RBlock) -> String {
        let mut out = String::new();
        match block {
            RBlock::Properties(entries) => {
                for p in entries {
                    match &p.value {
                        silica_formats::properties::Value::Text(s) => out.push_str(s),
                        silica_formats::properties::Value::List(items) => {
                            for item in items {
                                out.push_str(item);
                                out.push('\n');
                            }
                        }
                        _ => {}
                    }
                }
            }
            RBlock::Paragraph(segments) => {
                for seg in segments {
                    if let Segment::Text(rt) = seg {
                        out.push_str(&rt.text);
                    }
                }
            }
            RBlock::Heading { text, .. } => out.push_str(&text.text),
            RBlock::ThematicBreak => out.push_str("---"),
            RBlock::Code { text, .. } => out.push_str(text),
            RBlock::Html { text: rt, .. } => out.push_str(&rt.text),
            RBlock::Quote(children) => {
                for child in children {
                    out.push_str("> ");
                    out.push_str(&self.block_text(child));
                    out.push('\n');
                }
            }
            RBlock::Callout {
                title, children, ..
            } => {
                out.push_str(&title.text);
                out.push('\n');
                for child in children {
                    out.push_str(&self.block_text(child));
                }
            }
            RBlock::List { items, .. } => {
                for item in items {
                    let marker = match item.task {
                        Some(' ') => "[ ] ",
                        Some('x') => "[x] ",
                        _ => "",
                    };
                    out.push_str(marker);
                    for child in &item.children {
                        out.push_str(&self.block_text(child));
                    }
                    out.push('\n');
                }
            }
            RBlock::Table { header, rows, .. } => {
                for rt in header {
                    out.push_str(&rt.text);
                    out.push_str(" | ");
                }
                out.push('\n');
                for row in rows {
                    for rt in row {
                        out.push_str(&rt.text);
                        out.push_str(" | ");
                    }
                    out.push('\n');
                }
            }
            RBlock::Math(_) => {}
            RBlock::Mermaid { .. } => {}
            RBlock::Footnotes(notes) => {
                for (_, blocks) in notes {
                    for block in blocks {
                        out.push_str(&self.block_text(block));
                    }
                }
            }
        }
        out
    }

    /// The byte range `[start, end)` of text inside `block`, looking only at
    /// RichText runs (approximate at block level: returns whole runs that
    /// intersect the range).
    fn rich_text_in_block(&self, block: &RBlock, start: usize, end: usize) -> Option<String> {
        let mut out = String::new();
        match block {
            RBlock::Paragraph(segments) => {
                for seg in segments {
                    if let Segment::Text(rt) = seg {
                        if rt.text.get(start..end).is_some() {
                            out.push_str(&rt.text[start..end]);
                        } else if start < rt.text.len() {
                            out.push_str(&rt.text[start..]);
                        }
                    }
                }
            }
            RBlock::Heading { text, .. } => {
                return text.text.get(start..end).map(|s| s.to_string());
            }
            RBlock::Html { text: rt, .. } => {
                return rt.text.get(start..end).map(|s| s.to_string());
            }
            _ => {}
        }
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }

    /// Plays (or pauses, or seeks) a video embed clicked in the note.
    ///
    /// `share` of `Some(n)` is a click on the position bar; `None` is the
    /// picture or the button, which toggles.
    fn video_clicked(&self, path: &std::path::Path, share: Option<f32>, cx: &mut Context<Self>) {
        let path = path.to_path_buf();
        let open = {
            let video = self.video.borrow();
            video
                .as_ref()
                .filter(|v| v.path == path)
                .map(|v| (v.playing, v.info, v.at))
        };
        match (open, share) {
            // * A seek on the playing file.
            (Some((_, info, _)), Some(share)) => {
                self.video_start(path, info, info.duration * share.clamp(0.0, 1.0), cx)
            }
            // * The button or the picture: pause, or resume where it stopped.
            (Some((true, _, _)), None) => self.video_pause(&path, cx),
            (Some((false, info, at)), None) => {
                // * A file that played to its end starts again from the top,
                //   as a player's button does.
                let at = if info.duration > 0.0 && at >= info.duration - 0.05 {
                    0.0
                } else {
                    at
                };
                self.video_start(path, info, at, cx)
            }
            // * Nothing open yet.
            (None, share) => match crate::video::probe(&path) {
                Some(info) => {
                    let at = share.map_or(0.0, |s| info.duration * s.clamp(0.0, 1.0));
                    self.video_start(path, info, at, cx);
                }
                // * Not a video after all: the system player, as before.
                None => cx.open_url(&format!("file://{}", path.display())),
            },
        }
    }

    /// Starts `path` at `at`, replacing whatever was open. The task owns the
    /// decoder, so replacing a video kills the old `ffmpeg` by dropping it.
    fn video_start(
        &self,
        path: PathBuf,
        info: crate::video::Info,
        at: f32,
        cx: &mut Context<Self>,
    ) {
        if let Some(old) = self.video.borrow_mut().take() {
            crate::video::drop_frame(old.frame, cx);
        }
        let frame = crate::video::still(&path, &info, at);
        *self.video.borrow_mut() = Some(VideoState {
            path: path.clone(),
            info,
            playing: true,
            at,
            frame,
            task: None,
        });
        let task = Self::play_loop(
            cx.entity().downgrade(),
            self.video.clone(),
            path,
            info,
            at,
            cx,
        );
        if let Some(video) = self.video.borrow_mut().as_mut() {
            video.task = Some(task);
        }
        cx.notify();
    }

    /// Pauses the video that is open; its task ends and `ffmpeg` with it.
    fn video_pause(&self, path: &std::path::Path, cx: &mut Context<Self>) {
        if let Some(video) = self.video.borrow_mut().as_mut().filter(|v| v.path == path) {
            video.playing = false;
            video.task = None;
        }
        cx.notify();
    }

    /// The read loop: one frame at a time, installed only while the cell
    /// still holds this file and is still playing. Audio is decoded by a
    /// second ffmpeg process and played through rodio in sync with the
    /// frames when an audio output device is available.
    fn play_loop(
        view: gpui::WeakEntity<ReadingView>,
        slot: Rc<RefCell<Option<VideoState>>>,
        path: PathBuf,
        info: crate::video::Info,
        start: f32,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        cx.spawn(async move |_, cx| {
            let mut at = start;
            let Some(decoder) = crate::video::Decoder::spawn(&path, &info, start) else {
                return;
            };
            let mut decoder = Some(decoder);
            // Audio is optional: without ffmpeg audio output or rodio, video
            // still plays silently.
            let mut audio_decoder = crate::video::AudioDecoder::spawn(&path, start);
            let sink = match rodio::OutputStream::try_default() {
                Ok((_stream, handle)) => match rodio::Sink::try_new(&handle) {
                    Ok(sink) => {
                        sink.set_volume(1.0);
                        Some(sink)
                    }
                    Err(_) => None,
                },
                Err(_) => None,
            };
            loop {
                let wanted = slot
                    .borrow()
                    .as_ref()
                    .is_some_and(|v| v.path == path && v.playing);
                if !wanted {
                    break;
                }
                let Some(mut taken) = decoder.take() else {
                    break;
                };
                let mut audio_taken = audio_decoder.take();
                let (frame, back, audio_chunk, audio_back) = cx
                    .background_executor()
                    .spawn(async move {
                        let frame = taken.next_frame();
                        let chunk = audio_taken.as_mut().and_then(|d| d.next_chunk());
                        (frame, taken, chunk, audio_taken)
                    })
                    .await;
                decoder = Some(back);
                audio_decoder = audio_back;
                // Feed decoded PCM audio to rodio so the soundtrack plays.
                if let Some(sink) = &sink {
                    if let Some(chunk) = audio_chunk {
                        if !chunk.is_empty() {
                            sink.append(crate::video::PcmSource::new(chunk));
                        }
                    }
                }
                let Some(frame) = frame else {
                    // * End of file: the last frame stays up, stopped.
                    if let Some(video) = slot.borrow_mut().as_mut().filter(|v| v.path == path) {
                        video.playing = false;
                        video.at = info.duration;
                    }
                    view.update(cx, |_, cx| cx.notify()).ok();
                    break;
                };
                at += 1.0 / info.fps.max(1.0);
                let mut stale_to_drop = None;
                let installed = match slot
                    .borrow_mut()
                    .as_mut()
                    .filter(|v| v.path == path && v.playing)
                {
                    Some(video) => {
                        // * A frame's texture outlives its frame unless gpui is
                        //   told; the old one goes with the new one's arrival.
                        if let Some(stale) = video.frame.replace(frame) {
                            stale_to_drop = Some(stale);
                        }
                        video.at = at;
                        true
                    }
                    None => false,
                };
                view.update(cx, |_, cx| {
                    crate::video::drop_frame(stale_to_drop.take(), cx);
                    cx.notify();
                })
                .ok();
                if !installed {
                    break;
                }
            }
            if let Some(mut decoder) = decoder {
                decoder.stop();
            }
            if let Some(mut audio_decoder) = audio_decoder {
                audio_decoder.stop();
            }
            drop(sink);
        })
    }

    pub fn set_note(&mut self, path: Option<String>, model: Model, cx: &mut Context<Self>) {
        let same = same_note(self.path.as_deref(), path.as_deref());
        let top = self.list.logical_scroll_top();
        self.path = path;
        self.list.reset(model.blocks.len());
        let real_model = !model.blocks.is_empty();
        if same {
            // * A reload after a change on disk keeps the reader's place.
            self.list.scroll_to(top);
        }
        self.model = Arc::new(model);
        if !same {
            // * Another note: whatever was playing belongs to the old one.
            if let Some(old) = self.video.borrow_mut().take() {
                crate::video::drop_frame(old.frame, cx);
            }
            // * ... and the folds they opened: the watcher re-enters this on
            //   every autosave, so clearing them unconditionally collapsed an
            //   expanded callout 1.5 s after any save of the note being read.
            self.toggled.clear();
            self.folds.clear();
            if real_model {
                // * The real model landed: restore this note's saved folds.
                self.load_folds();
            }
        } else if real_model && self.folds.is_empty() {
            // * A placeholder model was replaced by the real one for the
            //   same note (a mode switch into Reading): restore what was
            //   saved for it.
            self.load_folds();
        }
        self.recompute_hidden();
        self.copied = None;
        cx.notify();
    }

    /// The saved folded headings of the note on screen, by their index in
    /// the view's heading order. A heading that moved or went away simply
    /// fails to resolve and stays unfolded; nothing is written to the note.
    fn load_folds(&mut self) {
        let Some(path) = self.path.as_deref() else {
            return;
        };
        let heads = self.headings();
        self.folds = self
            .fold_store
            .get(path)
            .map(|saved| {
                heads
                    .iter()
                    .enumerate()
                    .filter_map(|(ix, (_, _, key))| saved.contains(&ix).then_some(key.clone()))
                    .collect()
            })
            .unwrap_or_default();
    }

    /// The indexes of this note's currently folded headings, written to
    /// the store outside the vault whenever a fold changes. An empty set
    /// drops the note's entry.
    fn save_folds(&mut self) {
        let Some(path) = self.path.as_deref() else {
            return;
        };
        let heads = self.headings();
        let saved: HashSet<usize> = heads
            .iter()
            .enumerate()
            .filter(|(_, (_, _, key))| self.folds.contains(key))
            .map(|(ix, _)| ix)
            .collect();
        self.fold_store.put(path, saved);
    }

    // ----- heading folds --------------------------------------------------------

    /// Every top-level heading: its block index, level and key.
    fn headings(&self) -> Vec<(usize, u8, FoldKey)> {
        let heads: Vec<(usize, u8, &str)> = self
            .model
            .blocks
            .iter()
            .enumerate()
            .filter_map(|(ix, b)| match b {
                RBlock::Heading { level, plain, .. } => Some((ix, *level, plain.as_str())),
                _ => None,
            })
            .collect();
        let keys = fold::heading_keys(heads.iter().map(|(_, l, t)| (*l, *t)));
        heads
            .into_iter()
            .zip(keys)
            .map(|((ix, level, _), key)| (ix, level, key))
            .collect()
    }

    /// Marks the blocks a collapsed heading hides: from the block after it
    /// to the one before the next heading of the same or a higher level —
    /// absent from the view, as Obsidian's are absent from the DOM.
    fn recompute_hidden(&mut self) {
        let heads = self.headings();
        let mut hidden = vec![false; self.model.blocks.len()];
        for (i, (ix, level, key)) in heads.iter().enumerate() {
            if !self.folds.contains(key) {
                continue;
            }
            let end = heads[i + 1..]
                .iter()
                .find(|(_, l, _)| l <= level)
                .map(|(next, _, _)| *next)
                .unwrap_or(hidden.len());
            for h in hidden.iter_mut().take(end).skip(ix + 1) {
                *h = true;
            }
        }
        self.hidden = hidden;
    }

    fn toggle_heading(&mut self, key: FoldKey, cx: &mut Context<Self>) {
        match self.folds.iter().position(|k| *k == key) {
            Some(at) => {
                self.folds.remove(at);
            }
            None => self.folds.push(key),
        }
        self.recompute_hidden();
        self.save_folds();
        cx.notify();
    }

    /// The folds by key, for the editor to take over on a mode switch.
    pub fn fold_keys(&self) -> Vec<FoldKey> {
        self.folds.clone()
    }

    pub fn set_fold_keys(&mut self, keys: Vec<FoldKey>, cx: &mut Context<Self>) {
        self.folds = keys;
        self.recompute_hidden();
        self.save_folds();
        cx.notify();
    }

    pub fn set_fold_headings(&mut self, on: bool) {
        self.fold_headings = on;
    }

    pub fn properties_folded(&self) -> bool {
        self.toggled.contains(&PROPERTIES_FOLD_ID)
    }

    pub fn set_properties_folded(&mut self, on: bool, cx: &mut Context<Self>) {
        if on != self.properties_folded() {
            self.toggle_callout(PROPERTIES_FOLD_ID, cx);
        }
    }

    /// `editor:toggle-fold-properties` run from a preview leaf, which
    /// Obsidian honours.
    pub fn toggle_properties_fold(&mut self, cx: &mut Context<Self>) {
        self.toggle_callout(PROPERTIES_FOLD_ID, cx);
    }

    /// Scrolls so the top-level block `ix` is at the top of the view.
    /// The note on screen, as built.
    pub fn model(&self) -> &Model {
        &self.model
    }

    pub fn scroll_to_block(&mut self, ix: usize, cx: &mut Context<Self>) {
        // * A jump into a collapsed section expands it, as a link into a
        //   folded section was observed doing in the editor.
        if self.hidden.get(ix).copied().unwrap_or(false) {
            let heads = self.headings();
            let hiding: Vec<FoldKey> = heads
                .iter()
                .enumerate()
                .filter(|(i, (at, level, _))| {
                    let end = heads[i + 1..]
                        .iter()
                        .find(|(_, l, _)| l <= level)
                        .map(|(next, _, _)| *next)
                        .unwrap_or(self.model.blocks.len());
                    *at < ix && ix < end
                })
                .map(|(_, (_, _, key))| key.clone())
                .collect();
            self.folds.retain(|k| !hiding.contains(k));
            self.recompute_hidden();
            self.save_folds();
        }
        if ix < self.model.blocks.len() {
            self.list.scroll_to(ListOffset {
                item_ix: ix,
                offset_in_item: px(0.),
            });
            cx.notify();
        }
    }

    pub fn scroll_to_offset(&mut self, offset: usize, cx: &mut Context<Self>) {
        if let Some(ix) = self.model.block_at(offset) {
            self.scroll_to_block(ix, cx);
        }
    }

    /// Byte offset of the top-level block at the top of the view, which
    /// is what the Outline marks in reading mode.
    pub fn top_offset(&self) -> Option<usize> {
        let top = self.list.logical_scroll_top();
        self.model.starts.get(top.item_ix).copied()
    }

    pub fn scroll_to_end(&mut self, cx: &mut Context<Self>) {
        self.list.scroll_to_end();
        cx.notify();
    }

    /// Scrolls by `distance` pixels, the way the wheel does. A distance past
    /// either end is left to the list: its layout pass walks the scroll top
    /// back until the content fills the view again, which is what
    /// [`ReadingView::scroll_to_end`] relies on too.
    fn scroll_by(&mut self, distance: f32, cx: &mut Context<Self>) {
        self.list.scroll_by(px(distance));
        cx.notify();
    }

    /// One page of the painted viewport, down or up.
    fn page(&mut self, down: bool, cx: &mut Context<Self>) {
        let height = f32::from(self.list.viewport_bounds().size.height);
        let dy = page_distance(height);
        self.scroll_by(if down { dy } else { -dy }, cx);
    }

    /// One rendered line, down or up.
    fn line(&mut self, down: bool, cx: &mut Context<Self>) {
        let dy = line_distance(cx.global::<Theme>().base_font_size);
        self.scroll_by(if down { dy } else { -dy }, cx);
    }

    fn scroll_page_down(&mut self, _: &ScrollPageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.page(true, cx);
    }

    fn scroll_page_up(&mut self, _: &ScrollPageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.page(false, cx);
    }

    fn scroll_line_down(&mut self, _: &ScrollLineDown, _: &mut Window, cx: &mut Context<Self>) {
        self.line(true, cx);
    }

    fn scroll_line_up(&mut self, _: &ScrollLineUp, _: &mut Window, cx: &mut Context<Self>) {
        self.line(false, cx);
    }

    fn scroll_to_top(&mut self, _: &ScrollToTop, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_to_block(0, cx);
    }

    fn scroll_to_bottom(&mut self, _: &ScrollToBottom, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_to_end(cx);
    }

    /// What the reader is looking at, for the control socket's state dump.
    /// The scroll position is the list's own: the top-level block at the top
    /// of the view and how far into it the view starts, which is what
    /// [`ReadingView::scroll_to_block`] takes back.
    pub fn describe(&self) -> Value {
        let top = self.list.logical_scroll_top();
        json!({
            "path": self.path,
            // * How many blocks the built model holds: the count is 0 while
            //   the note was last opened in an editing mode, which has not
            //   built a model at all, and is what a mode switch into
            //   Reading view rebuilds.
            "blocks": self.model.blocks.len(),
            "scroll": {
                "item": top.item_ix,
                "offset": f32::from(top.offset_in_item),
            },
            // * An embedded base draws no chrome of its own, so this is the
            //   only way one is visible from outside the app.
            "bases": crate::bases::embed::describe_all(&self.model.blocks),
            // * And an embedded PDF likewise: its pages are images and its
            //   box has no title, so the dump is what names it.
            "pdfs": crate::pdf::embed::describe_all(&self.model.blocks),
        })
    }

    /// Shows the check on a code block's copy button for as long as
    /// Obsidian shows its own: sampled every 100 ms from the click, its
    /// icon is `lucide-check` through 900 ms and back to `lucide-copy` at
    /// 1000.
    fn flash_copied(&mut self, id: usize, cx: &mut Context<Self>) {
        self.copied = Some(id);
        self.flash = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1000))
                .await;
            this.update(cx, |view, cx| {
                view.copied = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn toggle_callout(&mut self, id: usize, cx: &mut Context<Self>) {
        if !self.toggled.remove(&id) {
            self.toggled.insert(id);
        }
        cx.notify();
    }

    fn render_block(
        &mut self,
        ix: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.global::<Theme>().clone();
        let model = self.model.clone();
        let Some(block) = model.blocks.get(ix) else {
            return div().into_any_element();
        };
        if self.hidden.get(ix).copied().unwrap_or(false) {
            // * Collapsed content takes no room at all.
            return div().into_any_element();
        }
        // * The block's inner width: the view minus its padding, capped at the
        //   readable line length.
        let mut available = f32::from(self.width) - 64.;
        if self.readable_line_length {
            available = available.min(700.);
        }
        let view = cx.entity();
        let on_link: LinkHandler = Rc::new({
            let view = view.clone();
            move |target, window, cx| {
                let modifiers = window.modifiers();
                let new_tab = modifiers.control || modifiers.platform;
                view.update(cx, |_, cx| {
                    cx.emit(if new_tab {
                        ReadingEvent::OpenLinkInNewTab(target)
                    } else {
                        ReadingEvent::OpenLink(target)
                    })
                });
            }
        });
        let on_toggle: ToggleHandler = Rc::new({
            let view = view.clone();
            move |id, cx| {
                view.update(cx, |v, cx| v.toggle_callout(id, cx));
            }
        });
        let on_copy: CopyHandler = Rc::new({
            let view = view.clone();
            move |id, cx| {
                view.update(cx, |v, cx| v.flash_copied(id, cx));
            }
        });
        let on_task: TaskHandler = Rc::new({
            let view = view.clone();
            move |marker, cx| {
                view.update(cx, |v, cx| {
                    if let Some(path) = v.path.clone() {
                        cx.emit(TaskToggled { path, marker });
                    }
                });
            }
        });
        // * A click inside a video embed calls back into this view, which owns
        //   the player.
        let view_for_video = view.clone();
        let on_video: VideoHandler = Rc::new(move |path, share, _window, cx| {
            let path = path.to_path_buf();
            view_for_video.update(cx, |this, cx| this.video_clicked(&path, share, cx));
        });
        let host = ReadingHost {
            properties: self.properties_in_document,
            copied: self.copied,
            on_copy,
            on_task,
            preview_resolver: self.preview_resolver.clone(),
            on_video,
            video: self.video.clone(),
            on_canvas: self
                .canvas_host
                .clone()
                .unwrap_or_else(|| Rc::new(missing_canvas)),
        };
        // * The reading view's properties block is now editable: clicks on
        //   rows come back as PropertyChanged events that the workspace
        //   writes to the vault file.
        let view_for_prop = view.clone();
        let on_property_action: PropertyHandler = Rc::new(move |action, _window, cx| {
            view_for_prop.update(cx, |this, cx| this.handle_property_action(action, cx));
        });
        let properties_host = PropertiesHost {
            field: self.property_field.clone(),
            selected_pill: self.selected_pill,
            mode: self.properties_in_document,
            menu: self.property_menu.clone(),
            on_action: on_property_action,
        };
        let r = Renderer::new(
            &theme,
            _window.text_system().clone(),
            available,
            &self.toggled,
            &self.property_types,
            Some(&properties_host),
            None,
            on_link,
            on_toggle,
            self.path.clone().unwrap_or_default(),
            view.downgrade(),
            self.selection.as_deref(),
        )
        .with_reading_host(&host);
        // * Remember the entries of the properties block so a pill remove,
        //   retype or item edit can look up the row it acts on.
        if let RBlock::Properties(entries) = block {
            self.property_entries = Some(entries.clone());
        }
        let mut inner = r.block(block, false);
        // * A heading carries Obsidian's `heading-collapse-indicator`: a
        //   chevron in the margin to its left, shown on hover or while
        //   collapsed, that only its own click toggles (the heading's text
        //   and the space beside it were observed doing nothing). `foldHeading`
        //   off hides the affordance. The heading is drawn here rather than
        //   by the renderer so that its margins sit outside the box the
        //   chevron is placed in.
        if let RBlock::Heading { level, text, .. } = block {
            if self.fold_headings {
                if let Some((_, _, key)) = self.headings().into_iter().find(|(at, _, _)| *at == ix)
                {
                    let collapsed = self.folds.contains(&key);
                    let base = theme.base_font_size;
                    let (size, weight, line_height) = crate::theme::heading_style(*level);
                    let top = match level {
                        1 => 1.0,
                        2 => 2.5,
                        _ => 1.5,
                    };
                    let line_px = base * size * line_height;
                    let view = view.clone();
                    // * The box reaches 28 px into the left margin, where the
                    //   chevron sits, so that hovering the chevron itself
                    //   counts as hovering the heading; the padding puts the
                    //   text back where it was.
                    inner = div()
                        .mt(px(base * top))
                        .mb(px(base))
                        .ml(px(-28.))
                        .pl(px(28.))
                        .relative()
                        .group("heading")
                        .child(
                            div()
                                .text_size(px(base * size))
                                .line_height(px(line_px))
                                .font_weight(FontWeight(weight))
                                .child(r.rich_at(text, base * size)),
                        )
                        .child(
                            div()
                                .id(("heading-fold", ix))
                                .absolute()
                                .left(px(0.))
                                .top(px((line_px - 16.) / 2.))
                                .size(px(16.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .when(!collapsed, |d| d.invisible())
                                .group_hover("heading", |s| s.visible())
                                .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                                    let key = key.clone();
                                    view.update(cx, |v, cx| v.toggle_heading(key, cx));
                                })
                                .child(icon(
                                    if collapsed {
                                        "chevron-right"
                                    } else {
                                        "chevron-down"
                                    },
                                    16.,
                                    theme.text_muted,
                                )),
                        )
                        .into_any_element();
                }
            }
        }
        let last = ix + 1 == model.blocks.len();
        let view_down = view.clone();
        let view_move = view.clone();
        let view_up = view.clone();
        div()
            .w_full()
            .flex()
            .flex_row()
            .justify_center()
            .px(px(32.))
            .when(ix == 0, |d| d.pt(px(24.)))
            .when(last, |d| d.pb(px(120.)))
            .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                view_down.update(cx, |v, cx| {
                    v.drag_start = Some((ix, 0));
                    v.selection = Some(vec![(ix, 0, 0, 0)]);
                    cx.notify();
                });
            })
            .on_mouse_move(move |_, _, cx| {
                view_move.update(cx, |v, cx| {
                    if v.drag_start.is_some() {
                        if let Some(ranges) = v.selection.as_mut() {
                            let already = ranges.iter().any(|(b, _, _, _)| *b == ix);
                            if !already {
                                ranges.push((ix, 0, 0, 0));
                            }
                        }
                        cx.notify();
                    }
                });
            })
            .on_mouse_up(gpui::MouseButton::Left, move |_, _, cx| {
                view_up.update(cx, |v, cx| {
                    v.drag_start = None;
                    cx.notify();
                });
            })
            .child(
                div()
                    .w_full()
                    .when(self.readable_line_length, |d| d.max_w(px(700.)))
                    .child(inner),
            )
            .into_any_element()
    }

    fn render_inline_title(&mut self, base: f32, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.global::<Theme>().clone();
        let name = self
            .path
            .as_deref()
            .and_then(|p| p.rsplit('/').next())
            .unwrap_or_default()
            .trim_end_matches(".md")
            .to_string();
        if self.inline_title_edit.is_some() {
            // * Editing: a focused text field. Key events are handled by
            //   inline_title_key_down on the root div.
            div()
                .id("inline-title")
                .px(px(base * 2.))
                .pt(px(base * 1.6))
                .pb(px(base * 0.4))
                .text_size(px(base * 1.6))
                .text_color(theme.text)
                .font_weight(gpui::FontWeight::BOLD)
                .cursor_pointer()
                .bg(theme.bg_active)
                .rounded_sm()
                .px_1()
                .child(SharedString::from(
                    self.inline_title_edit.as_deref().unwrap_or(&name),
                ))
                .into_any_element()
        } else {
            // * Static label: a single click starts editing.
            let view = cx.entity();
            div()
                .id("inline-title")
                .px(px(base * 2.))
                .pt(px(base * 1.6))
                .pb(px(base * 0.4))
                .text_size(px(base * 1.6))
                .text_color(theme.text)
                .font_weight(gpui::FontWeight::BOLD)
                .cursor_pointer()
                .child(SharedString::from(name.clone()))
                .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                    view.update(cx, |v, cx| {
                        v.inline_title_edit = Some(name.clone());
                        cx.notify();
                    });
                })
                .into_any_element()
        }
    }

    /// Handles key events while the inline title is being edited. Enter
    /// commits the rename, Escape abandons, everything else edits the text.
    fn inline_title_key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.inline_title_edit.is_none() {
            return;
        }
        cx.stop_propagation();
        let key = event.keystroke.key.clone();
        match key.as_str() {
            "Enter" => {
                let new_name = self.inline_title_edit.take().unwrap_or_default();
                let path = self.path.clone().unwrap_or_default();
                cx.emit(ReadingEvent::RenameFile {
                    path,
                    new_name: new_name.trim().to_string(),
                });
            }
            "Escape" => {
                self.inline_title_edit = None;
            }
            "Backspace" => {
                if let Some(text) = &mut self.inline_title_edit {
                    text.pop();
                }
            }
            s if s.len() == 1 => {
                if let Some(text) = &mut self.inline_title_edit {
                    text.push_str(s);
                }
            }
            _ => {}
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{line_distance, page_distance, same_note, LINE_HEIGHT};

    #[test]
    fn a_page_is_the_viewport_less_a_strip_of_overlap() {
        // * Tall viewport: the overlap is the 40 px cap.
        assert_eq!(page_distance(800.), 760.);
        // * Short viewport: the overlap is an eighth of it, so the page keys
        //   still move most of a view rather than a fixed 40 px.
        assert_eq!(page_distance(160.), 140.);
        // * The cap and the eighth meet at 320 px.
        assert_eq!(page_distance(320.), 280.);
    }

    #[test]
    fn a_page_never_scrolls_backwards_and_an_unpainted_view_not_at_all() {
        assert_eq!(page_distance(0.), 0.);
        assert_eq!(page_distance(-1.), 0.);
        for viewport in [1., 10., 41., 320., 1000., 4000.] {
            let d = page_distance(viewport);
            assert!(d > 0., "{viewport} scrolled {d}");
            assert!(d <= viewport, "{viewport} scrolled {d}, past a page");
            assert!(
                d >= viewport * 0.875,
                "{viewport} scrolled only {d}, less than seven eighths"
            );
        }
    }

    #[test]
    fn a_line_is_the_rendered_line_height() {
        assert_eq!(line_distance(16.), 16. * LINE_HEIGHT);
        assert_eq!(line_distance(20.), 32.);
        // * An arrow key moves less than a page key for every viewport a
        //   window can have.
        assert!(line_distance(16.) < page_distance(200.));
    }

    #[test]
    fn only_a_reload_of_the_same_note_is_the_same_note() {
        // * The reload the watcher drives on every autosave: the reader's
        //   place and their folds are kept.
        assert!(same_note(Some("Notes/A.md"), Some("Notes/A.md")));
        // * Another note, or no note: both are dropped.
        assert!(!same_note(Some("Notes/A.md"), Some("Notes/B.md")));
        assert!(!same_note(Some("Notes/A.md"), None));
        assert!(!same_note(None, Some("Notes/A.md")));
        // * Clearing the view twice is not a reload of a note.
        assert!(!same_note(None, None));
    }
}
