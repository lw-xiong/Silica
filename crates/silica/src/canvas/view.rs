//! The canvas element: paints a `.canvas` file and takes the mouse and the
//! keyboard over it.
//!
//! Everything that is not gpui lives in `silica_canvas` — the viewport
//! math, hit testing, snapping, the `Scene` with its selection and undo
//! history. This file is the glue: one `canvas()` element that lays out each
//! visible card's content (from `Cards`) in prepaint, paints the dot grid,
//! groups, edges, card frames, content, selection and handles in paint, and
//! registers the mouse listeners; a key context for the shortcuts; and the
//! small gpui trees around it (the zoom controls, the colour palette, the
//! parse error). Persistence is the workspace's: the view says when it was
//! edited and hands over `to_json()`; the workspace owns the timer and the
//! vault's write path, as it does for the editor.
//!
//! Sections: state, input, painting, persistence.

use crate::assets::icon;
use crate::canvas::cards::{Cards, TaskOrigin};
use crate::canvas::menu::{self, IconItem, ListItem};
use crate::editor::{Editor, EditorEvent};
use crate::reading::TaskMarker;
use crate::theme::Theme;
use crate::workspace::VaultState;
use gpui::{
    actions, canvas, div, fill, point, prelude::*, px, size, AnyElement, App, AvailableSpace,
    Bounds, ClipboardItem, Context, Corners, CursorStyle, DispatchPhase, Edges, Entity,
    EventEmitter, FocusHandle, Focusable, HitboxBehavior, Hsla, KeyBinding, KeyDownEvent,
    KeyUpEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder, PinchEvent,
    Pixels, Render, ScrollWheelEvent, SharedString, Subscription, TextAlign, Window,
};
use serde_json::{json, Value};
use silica_canvas::{
    anchor, arrow_head, edge_ends, edge_path_points, edge_stub, facing_side, hit_edge, hit_handle,
    hit_node, nodes_in, side_normal, snap, visible_nodes, Color, Handle, Point as CPoint, Rect,
    Scene, Selection, Viewport, ARROW, BORDER_WIDTH, CARD_RADIUS, EDGE_HIT, EDGE_TOLERANCE,
    EDGE_WIDTH, HANDLE, MIN_NODE, NUDGE, NUDGE_LARGE, WHEEL_ZOOM, ZOOM_MAX, ZOOM_MIN,
};
use silica_formats::canvas::{Canvas, Edge, Node, NodeType, Side};
use std::sync::Arc;

// ---- constants ----------------------------------------------------------------
// Screen measurements observed in Obsidian 1.14.4 on 2026-10-21 (see
// `docs/silica/Specification.md`, "Observed canvas view behavior"). Labels
// and edges scale with the square root of the zoom: Obsidian draws them in
// canvas space and counter-scales them by `1 / sqrt(scale)`.

/// Padding around the content when the view frames it (Shift+1).
const FIT_PADDING: f64 = 40.0;
/// Zoom change per control button press, in log2.
const ZOOM_STEP: f64 = 0.5;
/// A press that travels less than this is a click, not a drag.
const DRAG_THRESHOLD: f64 = 3.0;
/// The dot pattern doubles its spacing while it would fall below this on
/// screen: 20 units at scale 1, 20 px again at scale 0.5, 14.14 px at 0.177.
const MIN_DOT_SPACING: f64 = 14.142_135_623_730_951;
/// A dot's radius, screen pixels, at every zoom.
const DOT_RADIUS: f32 = 0.7;
/// The connector dot drawn at a hovered card's side midpoints (Silica's
/// own: Obsidian's connection points are invisible 20-unit squares).
const CONNECTOR_RADIUS: f32 = 4.0;
const CONNECTOR_HIT: f64 = 10.0;
/// The name above a file or link card: 15 units, counter-scaled.
const CARD_LABEL_SIZE: f64 = 15.0;
/// A group's label pill: 22.5 units of text, padded 4 by 8, radius 4.
const GROUP_LABEL_SIZE: f64 = 22.5;
const GROUP_LABEL_PADDING: (f64, f64) = (8.0, 4.0);
/// An edge's label pill: 20 units of text, 26 of line, padded 6, radius 4.
const EDGE_LABEL_SIZE: f64 = 20.0;
const EDGE_LABEL_LINE: f64 = 26.0;
const EDGE_LABEL_PADDING: f64 = 6.0;
const LABEL_RADIUS: f64 = 4.0;
/// Line height of the single-line labels relative to their font size.
const LABEL_LINE_HEIGHT: f64 = 1.3;
/// The gap between a label's bottom and the frame it names, canvas units
/// (counter-scaled).
const LABEL_GAP: f64 = 4.0;
/// Labels smaller than this on screen are not drawn.
const MIN_LABEL_PX: f64 = 4.0;
/// The ring around a selected card, canvas units, outside the border.
const SELECTION_RING: f64 = 2.0;
/// The control stack in the top-right corner: 32 px buttons in 1 px
/// bordered groups, 8 px from the corner.
const CONTROL_BUTTON: f32 = 32.0;
const CONTROL_MARGIN: f32 = 8.0;
const CONTROL_GAP: f32 = 6.0;
/// The card menu at the bottom centre: three 48 px buttons, 16 px up.
const CARD_MENU_BUTTON: f32 = 48.0;
const CARD_MENU_BOTTOM: f32 = 16.0;
/// How far above the selection its floating menu sits.
const MENU_RISE: f32 = 10.0;
/// The colour swatches of the "Set color" picker.
const SWATCH: f32 = 18.0;
const PALETTE_GAP: f32 = 6.0;
/// Space a group made around a selection leaves around its members, in
/// canvas units. OBSERVE.
const GROUP_PADDING: f64 = 40.0;
/// Where a duplicate lands relative to its original, in canvas units.
const DUPLICATE_OFFSET: f64 = silica_canvas::GRID;
/// How close to a group's frame a press counts as the frame: a group's
/// body is background, only its border and label take the pointer.
const GROUP_EDGE_HIT: f64 = 8.0;

// ---- actions ------------------------------------------------------------------

actions!(
    canvas_view,
    [
        DeleteSelection,
        SelectAll,
        Undo,
        Redo,
        ClearSelection,
        ZoomIn,
        ZoomOut,
        ResetZoom,
        ZoomToFit,
        ZoomToSelection,
        Copy,
        Cut,
        Paste,
        Duplicate,
        GroupSelection,
        NudgeLeft,
        NudgeRight,
        NudgeUp,
        NudgeDown,
        NudgeLeftFar,
        NudgeRightFar,
        NudgeUpFar,
        NudgeDownFar,
        EditSelected
    ]
);

/// Key bindings for the canvas; called once at startup. The context is the
/// view's own, beneath the workspace's, so Ctrl+Z here undoes a card move
/// rather than reaching the editor's undo command.
pub fn bind_keys(cx: &mut App) {
    let ctx = Some("CanvasView");
    cx.bind_keys([
        KeyBinding::new("delete", DeleteSelection, ctx),
        KeyBinding::new("backspace", DeleteSelection, ctx),
        KeyBinding::new("ctrl-a", SelectAll, ctx),
        KeyBinding::new("ctrl-z", Undo, ctx),
        KeyBinding::new("ctrl-shift-z", Redo, ctx),
        KeyBinding::new("ctrl-y", Redo, ctx),
        KeyBinding::new("escape", ClearSelection, ctx),
        KeyBinding::new("ctrl-=", ZoomIn, ctx),
        KeyBinding::new("ctrl--", ZoomOut, ctx),
        KeyBinding::new("ctrl-0", ResetZoom, ctx),
        KeyBinding::new("shift-1", ZoomToFit, ctx),
        KeyBinding::new("shift-2", ZoomToSelection, ctx),
        KeyBinding::new("ctrl-c", Copy, ctx),
        KeyBinding::new("ctrl-x", Cut, ctx),
        KeyBinding::new("ctrl-v", Paste, ctx),
        KeyBinding::new("ctrl-d", Duplicate, ctx),
        KeyBinding::new("ctrl-g", GroupSelection, ctx),
        KeyBinding::new("left", NudgeLeft, ctx),
        KeyBinding::new("right", NudgeRight, ctx),
        KeyBinding::new("up", NudgeUp, ctx),
        KeyBinding::new("down", NudgeDown, ctx),
        KeyBinding::new("shift-left", NudgeLeftFar, ctx),
        KeyBinding::new("shift-right", NudgeRightFar, ctx),
        KeyBinding::new("shift-up", NudgeUpFar, ctx),
        KeyBinding::new("shift-down", NudgeDownFar, ctx),
        KeyBinding::new("enter", EditSelected, ctx),
    ]);
}

// ---- state --------------------------------------------------------------------

/// What the view tells the workspace.
#[derive(Debug, Clone)]
pub enum CanvasEvent {
    /// The canvas changed and is dirty: the autosave timer starts.
    Edited,
    /// A link inside a card was clicked; the workspace follows it the way
    /// it follows one in the reading view.
    OpenLink(crate::reading::LinkTarget),
    /// The view moved or zoomed: the tab records this `viewState` for
    /// `workspace.json`.
    Viewport(Value),
    /// "Add note from vault" or "Add media from vault": the workspace opens
    /// its chooser over the vault's files and hands the pick back to
    /// [`CanvasView::add_file_card_at_center`].
    PickFile(FilePick),
}

/// Which files a vault pick offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilePick {
    Note,
    Media,
}

/// A mouse interaction in progress, from a press to its release.
#[derive(Debug, Clone)]
enum Drag {
    /// Middle button, or space and the left button: the view follows the pointer.
    Pan { last: CPoint },
    /// A left press on the background: the rectangle selects what it crosses.
    Marquee {
        start: CPoint,
        end: CPoint,
        /// Shift was held: the marquee adds to the selection.
        additive: bool,
    },
    /// A left press on a card: the selection moves with the pointer once it
    /// has travelled `DRAG_THRESHOLD`. `applied` is the canvas delta the
    /// selection has been moved by so far, snap correction included, so each
    /// event can move it to exactly `pointer - press`.
    Move {
        press: CPoint,
        applied: CPoint,
        /// The card the press landed on: the one the snap is computed from.
        primary: String,
        moved: bool,
    },
    /// A press on one of a selected card's handles.
    Resize {
        id: String,
        handle: Handle,
        origin: Rect,
        press: CPoint,
    },
    /// A press on a connector dot: an edge follows the pointer to a card.
    Connect {
        from: String,
        side: Side,
        to: CPoint,
    },
}

/// A card's content laid out for this frame: where it goes and what to paint there.
struct CardContent {
    /// The node's index in the canvas, which the paint passes key on.
    ix: usize,
    frame: Bounds<Pixels>,
    element: AnyElement,
}

/// What prepaint hands to paint.
struct Prepared {
    hitbox: gpui::Hitbox,
    contents: Vec<CardContent>,
}

pub struct CanvasView {
    pub scene: Scene,
    pub cards: Cards,
    /// The vault path on screen; `None` when nothing is loaded.
    path: Option<String>,
    /// The file's bytes as last read or written, so a watcher event for the
    /// view's own save is told from a change made by someone else.
    saved_text: String,
    /// Why the file could not be drawn, shown in place of the canvas.
    error: Option<String>,
    viewport: Viewport,
    /// False until the viewport has been set — from the leaf's `viewState`
    /// or a fit around the content, which needs the element's size and so
    /// waits for the first prepaint.
    framed: bool,
    bounds: Bounds<Pixels>,
    focus_handle: FocusHandle,
    state: Option<Arc<VaultState>>,
    /// The card under the pointer, which shows its connector dots.
    hovered: Option<String>,
    drag: Option<Drag>,
    /// Space is held: a left drag pans.
    space_held: bool,
    /// The viewport was set during prepaint (the first fit), where no
    /// event can be sent; the next frame announces it.
    announce_viewport: bool,
    /// Hook for text editing, which a later change supplies: the id of the
    /// card whose text is being edited. A double click on a text card sets
    /// it and Escape clears it; nothing draws an editor in it yet, and the
    /// workspace reads it from `describe()` only.
    pub editing: Option<String>,
    /// The right-click menu: where it was opened and what it is about.
    context_menu: Option<(gpui::Point<Pixels>, ContextTarget)>,
    /// The "Set color" swatches are open under the floating menu.
    palette_open: bool,
    /// Where each group's label pill was painted this frame, in window
    /// pixels, so a press on it selects the group (its body is background).
    label_hits: std::cell::RefCell<Vec<(String, Bounds<Pixels>)>>,
    /// The editor open in the card named by `editing`, made and dropped by
    /// `sync_editor` so every way into and out of editing is one field.
    card_editor: Option<CardEditor>,
}

/// A text card being typed into: a Live Preview editor laid out in the
/// card's frame. Its text is copied into the node on every edit, so the
/// autosave writes it while the session's single undo step stays open.
struct CardEditor {
    id: String,
    editor: Entity<Editor>,
    _subscriptions: Vec<Subscription>,
}

/// An edge's curve with its two anchors: the point on each card and the
/// side it is on.
type EdgeGeometry = (silica_canvas::EdgePath, (CPoint, Side), (CPoint, Side));

/// What a press found under the pointer.
enum Hit {
    Node(String),
    Edge(String),
}

/// What a right-click landed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContextTarget {
    Card,
    Cards,
    Edge,
    Background,
}

impl EventEmitter<CanvasEvent> for CanvasView {}

impl Focusable for CanvasView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl CanvasView {
    pub fn new(cx: &mut Context<Self>) -> CanvasView {
        let mut cards = Cards::new();
        // A card's link is the workspace's to follow; the view only relays it.
        let weak = cx.weak_entity();
        cards.set_on_link(std::rc::Rc::new(move |_, target, _, cx| {
            weak.update(cx, |_, cx| cx.emit(CanvasEvent::OpenLink(target)))
                .ok();
        }));
        // A task checkbox inside a text card flips the marker in the node's
        // own text, with the editor's rule (`[ ]` to `[x]`, anything else
        // to `[ ]`), and saves the canvas. One inside an embedded note would
        // have to write that note; no view writes a note it does not hold,
        // so that click stays inert for now.
        let weak = cx.weak_entity();
        cards.set_on_task(std::rc::Rc::new(move |origin, marker, cx| {
            if let TaskOrigin::Node(id) = origin {
                let id = id.clone();
                weak.update(cx, |view, cx| view.toggle_node_task(&id, marker, cx))
                    .ok();
            }
        }));
        CanvasView {
            scene: Scene::new(Canvas::default()),
            cards,
            path: None,
            saved_text: String::new(),
            error: None,
            viewport: Viewport::default(),
            framed: false,
            bounds: Bounds::default(),
            focus_handle: cx.focus_handle(),
            state: None,
            hovered: None,
            drag: None,
            space_held: false,
            announce_viewport: false,
            editing: None,
            context_menu: None,
            palette_open: false,
            label_hits: std::cell::RefCell::new(Vec::new()),
            card_editor: None,
        }
    }

    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    /// The element's size in logical pixels, as the viewport math wants it.
    fn size(&self) -> (f64, f64) {
        (
            f64::from(f32::from(self.bounds.size.width)),
            f64::from(f32::from(self.bounds.size.height)),
        )
    }

    fn scale(&self) -> f64 {
        self.viewport.scale()
    }

    /// A window point as a point in the element.
    fn local(&self, p: gpui::Point<Pixels>) -> CPoint {
        CPoint::new(
            f64::from(f32::from(p.x - self.bounds.origin.x)),
            f64::from(f32::from(p.y - self.bounds.origin.y)),
        )
    }

    fn to_canvas(&self, p: gpui::Point<Pixels>) -> CPoint {
        self.viewport.to_canvas(self.local(p), self.size())
    }

    /// A canvas rectangle in element pixels.
    fn screen_rect(&self, r: &Rect) -> Rect {
        self.viewport.rect_to_screen(*r, self.size())
    }

    /// Frames the whole canvas; an empty one shows the origin at scale 1.
    fn fit_all(&mut self) {
        self.viewport =
            Viewport::fit_nodes(self.scene.canvas.nodes.iter(), self.size(), FIT_PADDING)
                .unwrap_or_default();
        self.framed = true;
    }

    fn fit_selection(&mut self) {
        let nodes = self
            .scene
            .selection
            .nodes
            .iter()
            .filter_map(|id| self.scene.node(id));
        if let Some(vp) = Viewport::fit_nodes(nodes, self.size(), FIT_PADDING) {
            self.viewport = vp;
        }
    }

    /// The colour a node's or edge's `color` paints as, `None` for the
    /// theme's default (which differs between a card, an edge and a group).
    fn color_of(color: Option<&str>, theme: &Theme) -> Option<Hsla> {
        match Color::parse(color) {
            Color::Default => None,
            Color::Preset(n) => theme.canvas.preset(n),
            Color::Hex(v) => Some(gpui::rgb(v).into()),
        }
    }

    /// The factor labels and edges scale by: the square root of the zoom.
    fn label_scale(&self) -> f64 {
        self.scale().sqrt()
    }

    /// A card's or group's border colour: its colour at `border_alpha`, or
    /// the default colour solid.
    fn border_color(&self, node: &Node, theme: &Theme) -> Hsla {
        match Self::color_of(node.color.as_deref(), theme) {
            Some(mut c) => {
                c.a = theme.canvas.border_alpha;
                c
            }
            None => theme.canvas.card_border,
        }
    }

    /// The border width on screen: `BORDER_WIDTH` canvas units, never thinner
    /// than a pixel.
    fn border_px(&self) -> Pixels {
        px((f64::from(BORDER_WIDTH) * self.scale()).max(1.0) as f32)
    }

    fn viewport_changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(CanvasEvent::Viewport(self.view_state_json()));
        cx.notify();
    }

    /// Something changed in the scene: the workspace starts its save timer.
    fn edited(&mut self, cx: &mut Context<Self>) {
        if self.scene.is_dirty() {
            cx.emit(CanvasEvent::Edited);
        }
        cx.notify();
    }

    /// Flips the task marker at `marker` in a text card and saves.
    fn toggle_node_task(&mut self, id: &str, marker: TaskMarker, cx: &mut Context<Self>) {
        let Some(text) = self
            .scene
            .node(id)
            .and_then(|n| n.text.as_deref())
            .and_then(|t| toggle_task_in(t, marker.line, marker.col))
        else {
            return;
        };
        self.scene.begin();
        self.scene.set_text(id, &text);
        self.scene.commit();
        self.edited(cx);
    }

    // ---- editing a card ----

    /// Makes the editor match `editing`: opens one in the named card, or
    /// closes the one that is open. Called from `render`, which has the
    /// window the editor needs, so a double click, Enter, the menus and an
    /// edge dropped on empty space all just set `editing`.
    fn sync_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = self.editing.clone().filter(|id| {
            self.scene
                .node(id)
                .is_some_and(|n| n.kind == NodeType::Text)
        });
        if self.card_editor.as_ref().map(|c| &c.id) == wanted.as_ref() {
            return;
        }
        if self.card_editor.is_some() {
            self.close_editor(window, cx);
        }
        let Some(id) = wanted else {
            self.editing = None;
            return;
        };
        let text = self
            .scene
            .node(&id)
            .and_then(|n| n.text.clone())
            .unwrap_or_default();
        let name: SharedString = self.path.clone().unwrap_or_default().into();
        let editor = cx.new(|cx| Editor::new(window, cx));
        if let Some(state) = self.state.clone() {
            let embed_source: Arc<dyn crate::reading::EmbedSource + Send + Sync> = state.clone();
            let build_options = state.build_options();
            editor.update(cx, |e, cx| {
                e.set_vault(Some(embed_source), build_options, cx);
                e.set_suggest_vault(state.clone());
            });
        }
        editor.update(cx, |e, cx| {
            e.set_readable_line_length(false);
            e.set_live_preview(true, cx);
            e.set_text(name, text, cx);
        });
        let card = id.clone();
        let subscriptions = vec![cx.subscribe(
            &editor,
            move |this, editor, event: &EditorEvent, cx| match event {
                EditorEvent::Edited => {
                    let text = editor.read(cx).text().to_string();
                    this.scene.set_text(&card, &text);
                    this.scene.mark_dirty();
                    this.edited(cx);
                }
                EditorEvent::OpenLink(target) | EditorEvent::OpenLinkInNewTab(target) => {
                    cx.emit(CanvasEvent::OpenLink(target.clone()));
                }
                _ => {}
            },
        )];
        // The session is one undo step, closed when the editor goes.
        self.scene.begin();
        window.focus(&editor.focus_handle(cx), cx);
        self.card_editor = Some(CardEditor {
            id,
            editor,
            _subscriptions: subscriptions,
        });
    }

    /// Ends the editing session: the card keeps the final text, an emptied
    /// card is removed (observed: Obsidian deletes a card left empty), the
    /// step is committed and the keyboard comes back to the canvas.
    fn close_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(card) = self.card_editor.take() else {
            return;
        };
        let text = card.editor.read(cx).text().to_string();
        if text.trim().is_empty() {
            let keep = std::mem::take(&mut self.scene.selection);
            self.scene.selection = Selection::only_node(&card.id);
            self.scene.delete_selection();
            self.scene.selection = keep;
            self.scene.selection.retain_present(&self.scene.canvas);
        } else {
            self.scene.set_text(&card.id, &text);
        }
        self.scene.commit();
        if card.editor.read(cx).focus_handle_ref().is_focused(window) {
            window.focus(&self.focus_handle, cx);
        }
        self.edited(cx);
    }

    /// The editor open in a card, for the socket's typing.
    pub fn card_editor(&self) -> Option<&Entity<Editor>> {
        self.card_editor.as_ref().map(|c| &c.editor)
    }

    /// Enter on one selected text card opens it for editing.
    fn edit_selected(&mut self, _: &EditSelected, _: &mut Window, cx: &mut Context<Self>) {
        if let [id] = self.scene.selection.nodes.as_slice() {
            if self
                .scene
                .node(id)
                .is_some_and(|n| n.kind == NodeType::Text)
            {
                self.editing = Some(id.clone());
                cx.notify();
            }
        }
    }

    // ---- input: keyboard ----

    fn delete_selection(&mut self, _: &DeleteSelection, _: &mut Window, cx: &mut Context<Self>) {
        if self.scene.selection.is_empty() {
            return;
        }
        self.scene.begin();
        self.scene.delete_selection();
        self.scene.commit();
        self.edited(cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.scene.select_all();
        cx.notify();
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if self.scene.undo() {
            self.edited(cx);
        }
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if self.scene.redo() {
            self.edited(cx);
        }
    }

    fn clear_selection(&mut self, _: &ClearSelection, _: &mut Window, cx: &mut Context<Self>) {
        // Escape ends an editing session and leaves the card selected
        // (observed); a second Escape clears the selection.
        if self.editing.take().is_some() {
            cx.notify();
            return;
        }
        self.scene.selection.clear();
        cx.notify();
    }

    /// The selection goes to the clipboard as JSON Canvas text, which is
    /// what Obsidian puts there, so cards travel between the two apps and
    /// between canvases.
    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if self.scene.selection.nodes.is_empty() {
            return;
        }
        let text = self.scene.copy_selection().to_json();
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if self.scene.selection.nodes.is_empty() {
            return;
        }
        self.copy(&Copy, window, cx);
        self.delete_selection(&DeleteSelection, window, cx);
    }

    /// JSON Canvas text pastes as cards offset from their originals; any
    /// other text becomes one new card at the middle of the view.
    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let clip = Canvas::from_json(&text)
            .ok()
            .filter(|c| !c.nodes.is_empty());
        self.scene.begin();
        match clip {
            Some(clip) => {
                self.scene.paste(&clip, None);
            }
            None => {
                let center = self.viewport.to_canvas(
                    CPoint::new(self.size().0 / 2.0, self.size().1 / 2.0),
                    self.size(),
                );
                let at = silica_canvas::snap_point(CPoint::new(
                    center.x - silica_canvas::DEFAULT_CARD.0 / 2.0,
                    center.y - silica_canvas::DEFAULT_CARD.1 / 2.0,
                ));
                self.scene.add_text_card(at, text.trim_end());
            }
        }
        self.scene.commit();
        self.edited(cx);
    }

    fn duplicate(&mut self, _: &Duplicate, _: &mut Window, cx: &mut Context<Self>) {
        if self.scene.selection.nodes.is_empty() {
            return;
        }
        self.scene.begin();
        self.scene
            .duplicate_selection(CPoint::new(DUPLICATE_OFFSET, DUPLICATE_OFFSET));
        self.scene.commit();
        self.edited(cx);
    }

    fn group_selection(&mut self, _: &GroupSelection, _: &mut Window, cx: &mut Context<Self>) {
        if self.scene.selection.nodes.is_empty() {
            return;
        }
        self.scene.begin();
        self.scene.group_selection(GROUP_PADDING);
        self.scene.commit();
        self.edited(cx);
    }

    /// An arrow key moves the selection one grid step, Shift five, as
    /// Obsidian's view scope binds them.
    fn nudge(&mut self, dx: f64, dy: f64, cx: &mut Context<Self>) {
        if self.scene.selection.nodes.is_empty() {
            return;
        }
        self.scene.begin();
        self.scene.nudge_selection(dx, dy);
        self.scene.commit();
        self.edited(cx);
    }

    fn nudge_left(&mut self, _: &NudgeLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.nudge(-NUDGE, 0.0, cx);
    }
    fn nudge_right(&mut self, _: &NudgeRight, _: &mut Window, cx: &mut Context<Self>) {
        self.nudge(NUDGE, 0.0, cx);
    }
    fn nudge_up(&mut self, _: &NudgeUp, _: &mut Window, cx: &mut Context<Self>) {
        self.nudge(0.0, -NUDGE, cx);
    }
    fn nudge_down(&mut self, _: &NudgeDown, _: &mut Window, cx: &mut Context<Self>) {
        self.nudge(0.0, NUDGE, cx);
    }
    fn nudge_left_far(&mut self, _: &NudgeLeftFar, _: &mut Window, cx: &mut Context<Self>) {
        self.nudge(-NUDGE_LARGE, 0.0, cx);
    }
    fn nudge_right_far(&mut self, _: &NudgeRightFar, _: &mut Window, cx: &mut Context<Self>) {
        self.nudge(NUDGE_LARGE, 0.0, cx);
    }
    fn nudge_up_far(&mut self, _: &NudgeUpFar, _: &mut Window, cx: &mut Context<Self>) {
        self.nudge(0.0, -NUDGE_LARGE, cx);
    }
    fn nudge_down_far(&mut self, _: &NudgeDownFar, _: &mut Window, cx: &mut Context<Self>) {
        self.nudge(0.0, NUDGE_LARGE, cx);
    }

    /// Adds a text card at the middle of the view and hands it to the
    /// editor, as "Add card" does.
    fn add_card_at_center(&mut self, cx: &mut Context<Self>) {
        let (w, h) = silica_canvas::DEFAULT_CARD;
        let c = self.viewport.to_canvas(
            CPoint::new(self.size().0 / 2.0, self.size().1 / 2.0),
            self.size(),
        );
        self.scene.begin();
        let id = self.scene.add_text_card(
            CPoint::new((c.x - w / 2.0).round(), (c.y - h / 2.0).round()),
            "",
        );
        self.scene.commit();
        self.editing = Some(id);
        self.edited(cx);
    }

    /// Adds a card embedding a vault file at the middle of the view, from
    /// the pickers.
    pub fn add_file_card_at_center(&mut self, path: &str, cx: &mut Context<Self>) {
        let (w, h) = silica_canvas::DEFAULT_FILE_CARD;
        let c = self.viewport.to_canvas(
            CPoint::new(self.size().0 / 2.0, self.size().1 / 2.0),
            self.size(),
        );
        self.scene.begin();
        self.scene.add_file_card(
            CPoint::new((c.x - w / 2.0).round(), (c.y - h / 2.0).round()),
            path,
            None,
        );
        self.scene.commit();
        self.edited(cx);
    }

    /// `canvas:jump-to-group`: frames the group and selects it.
    pub fn jump_to_group(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.scene.node(id).is_none() {
            return;
        }
        self.scene.selection = Selection::only_node(id);
        self.zoom_to_node(id, cx);
    }

    /// The groups `canvas:jump-to-group` offers: label and id.
    pub fn group_targets(&self) -> Vec<(String, String)> {
        self.scene
            .jump_targets()
            .into_iter()
            .map(|n| (n.label.clone().unwrap_or_default(), n.id.clone()))
            .collect()
    }

    /// The one selected text card, for `canvas:convert-to-file`: its id and
    /// text.
    pub fn selected_text_card(&self) -> Option<(String, String)> {
        let [id] = self.scene.selection.nodes.as_slice() else {
            return None;
        };
        let node = self.scene.node(id)?;
        (node.kind == NodeType::Text).then(|| (id.clone(), node.text.clone().unwrap_or_default()))
    }

    /// Turns a text card into a file card over `path`, after the workspace
    /// wrote the note: the geometry and colour stay, the text goes.
    pub fn convert_to_file(&mut self, id: &str, path: &str, cx: &mut Context<Self>) {
        self.editing = None;
        self.scene.begin();
        if let Some(node) = self.scene.node_mut(id) {
            node.kind = NodeType::File;
            node.file = Some(path.to_string());
            node.text = None;
        }
        self.scene.commit();
        self.edited(cx);
    }

    /// Frames one node.
    fn zoom_to_node(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(n) = self.scene.node(id) {
            self.viewport = Viewport::fit(Rect::of(n), self.size(), FIT_PADDING);
            self.viewport_changed(cx);
        }
    }

    /// Cycles a selected edge's arrows the way Obsidian's "Line direction"
    /// menu orders them: unidirectional, bidirectional, nondirectional.
    fn cycle_edge_direction(&mut self, cx: &mut Context<Self>) {
        let ids = self.scene.selection.edges.clone();
        if ids.is_empty() {
            return;
        }
        self.scene.begin();
        for id in ids {
            let Some(e) = self.scene.edge(&id) else {
                continue;
            };
            let (from, to) = (!matches!(edge_ends(e), (_, false)), edge_ends(e).0);
            let (from_arrow, to_arrow) = match (edge_ends(e).0, edge_ends(e).1) {
                (false, true) => (true, true),
                (true, true) => (false, false),
                _ => (false, true),
            };
            let _ = (from, to);
            self.scene.set_edge_ends(&id, from_arrow, to_arrow);
        }
        self.scene.commit();
        self.edited(cx);
    }

    /// What a floating-menu button or a context-menu entry does.
    pub(crate) fn menu_pick(
        &mut self,
        id: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match id {
            "remove" => self.delete_selection(&DeleteSelection, window, cx),
            "color" => {
                self.palette_open = !self.palette_open;
                cx.notify();
            }
            "zoom" => self.zoom_to_selection(&ZoomToSelection, window, cx),
            "edit" => {
                if let [id] = self.scene.selection.nodes.as_slice() {
                    self.editing = Some(id.clone());
                    cx.notify();
                }
            }
            "direction" => self.cycle_edge_direction(cx),
            "remove-label" => {
                let ids = self.scene.selection.edges.clone();
                self.scene.begin();
                for id in ids {
                    self.scene.set_edge_label(&id, "");
                }
                self.scene.commit();
                self.edited(cx);
            }
            "group" => self.group_selection(&GroupSelection, window, cx),
            "cut" => self.cut(&Cut, window, cx),
            "copy" => self.copy(&Copy, window, cx),
            "paste" => self.paste(&Paste, window, cx),
            "undo" => self.undo(&Undo, window, cx),
            "add-card" => self.add_card_at_center(cx),
            "add-note" => cx.emit(CanvasEvent::PickFile(FilePick::Note)),
            "add-media" => cx.emit(CanvasEvent::PickFile(FilePick::Media)),
            "follow" => {
                let target = self
                    .scene
                    .selection
                    .edges
                    .first()
                    .and_then(|id| self.scene.edge(id))
                    .map(|e| e.to_node.clone());
                if let Some(to) = target {
                    self.scene.selection = Selection::only_node(&to);
                    self.zoom_to_node(&to, cx);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn close_context_menu(&mut self, cx: &mut Context<Self>) {
        if self.context_menu.take().is_some() {
            cx.notify();
        }
    }

    fn zoom_in(&mut self, _: &ZoomIn, _: &mut Window, cx: &mut Context<Self>) {
        self.viewport.zoom_step(ZOOM_STEP, self.size());
        self.viewport_changed(cx);
    }

    fn zoom_out(&mut self, _: &ZoomOut, _: &mut Window, cx: &mut Context<Self>) {
        self.viewport.zoom_step(-ZOOM_STEP, self.size());
        self.viewport_changed(cx);
    }

    /// Scale 1 around the point the view is looking at, Obsidian's "Reset zoom".
    fn reset_zoom(&mut self, _: &ResetZoom, _: &mut Window, cx: &mut Context<Self>) {
        self.viewport.zoom = 0.0;
        self.viewport_changed(cx);
    }

    fn zoom_to_fit(&mut self, _: &ZoomToFit, _: &mut Window, cx: &mut Context<Self>) {
        self.fit_all();
        self.viewport_changed(cx);
    }

    fn zoom_to_selection(&mut self, _: &ZoomToSelection, _: &mut Window, cx: &mut Context<Self>) {
        self.fit_selection();
        self.viewport_changed(cx);
    }

    // ---- input: mouse ----

    /// The handle under an element point when exactly one card is selected.
    fn handle_at(&self, local: CPoint) -> Option<(String, Handle, Rect)> {
        let [id] = self.scene.selection.nodes.as_slice() else {
            return None;
        };
        let node = self.scene.node(id)?;
        let rect = Rect::of(node);
        let handle = hit_handle(&self.screen_rect(&rect), local, HANDLE)?;
        Some((id.clone(), handle, rect))
    }

    /// The connector dot under an element point on the hovered card.
    fn connector_at(&self, local: CPoint) -> Option<(String, Side)> {
        let id = self.hovered.as_ref()?;
        let node = self.scene.node(id)?;
        if node.kind == NodeType::Group {
            return None;
        }
        let rect = self.screen_rect(&Rect::of(node));
        [Side::Top, Side::Right, Side::Bottom, Side::Left]
            .into_iter()
            .find(|side| anchor(&rect, *side).distance(local) <= CONNECTOR_HIT)
            .map(|side| (id.clone(), side))
    }

    /// The node a press lands on. A group counts only by its frame: a
    /// press inside its body is a press on the background (observed).
    fn node_at(&self, at: CPoint) -> Option<usize> {
        let ix = hit_node(&self.scene.canvas, at)?;
        let node = &self.scene.canvas.nodes[ix];
        if node.kind != NodeType::Group {
            return Some(ix);
        }
        let r = Rect::of(node);
        let band = GROUP_EDGE_HIT / self.scale();
        let inner = Rect::new(
            r.x + band,
            r.y + band,
            (r.width - 2.0 * band).max(0.0),
            (r.height - 2.0 * band).max(0.0),
        );
        if inner.contains(at) {
            // Something beneath the group's body? Groups come first in the
            // array, so a lower group's frame could still be there.
            return None;
        }
        Some(ix)
    }

    /// A card, else an edge, under the pointer.
    fn edge_or_node_at(&self, _local: CPoint, at: CPoint) -> Option<Hit> {
        if let Some(ix) = self.node_at(at) {
            return Some(Hit::Node(self.scene.canvas.nodes[ix].id.clone()));
        }
        let tolerance = EDGE_HIT / self.scale().sqrt();
        hit_edge(
            &self.scene.canvas,
            at,
            tolerance.max(EDGE_TOLERANCE / self.scale()),
        )
        .map(|ix| Hit::Edge(self.scene.canvas.edges[ix].id.clone()))
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
        let local = self.local(event.position);
        let at = self.to_canvas(event.position);
        if event.button == MouseButton::Middle
            || (event.button == MouseButton::Left && self.space_held)
        {
            self.drag = Some(Drag::Pan { last: local });
            cx.notify();
            return;
        }
        self.context_menu = None;
        if event.button == MouseButton::Right {
            // What is under the pointer joins the selection (a card outside
            // it replaces it), and the menu for that kind of thing opens.
            let target = if let Some(ix) = self.edge_or_node_at(local, at) {
                match ix {
                    Hit::Edge(id) => {
                        if !self.scene.selection.has_edge(&id) {
                            self.scene.selection = Selection::only_edge(&id);
                        }
                        ContextTarget::Edge
                    }
                    Hit::Node(id) => {
                        if !self.scene.selection.has_node(&id) {
                            self.scene.selection = Selection::only_node(&id);
                        }
                        if self.scene.selection.nodes.len() > 1 {
                            ContextTarget::Cards
                        } else {
                            ContextTarget::Card
                        }
                    }
                }
            } else {
                ContextTarget::Background
            };
            self.context_menu = Some((event.position, target));
            cx.notify();
            return;
        }
        if event.button != MouseButton::Left {
            return;
        }
        let shift = event.modifiers.shift;
        // A group's label pill selects (and drags) the group.
        let on_label = self
            .label_hits
            .borrow()
            .iter()
            .find(|(_, b)| b.contains(&event.position))
            .map(|(id, _)| id.clone());
        if let Some(id) = on_label {
            if shift {
                self.scene.selection.toggle_node(&id);
            } else if !self.scene.selection.has_node(&id) {
                self.scene.selection = Selection::only_node(&id);
            }
            self.scene.begin();
            self.drag = Some(Drag::Move {
                press: at,
                applied: CPoint::default(),
                primary: id,
                moved: false,
            });
            cx.notify();
            return;
        }
        // The connector dots sit where the side-midpoint handles do, and
        // on a hovered card they are what is drawn there, so they win.
        if let Some((from, side)) = self.connector_at(local) {
            self.drag = Some(Drag::Connect { from, side, to: at });
            cx.notify();
            return;
        }
        if let Some((id, handle, origin)) = self.handle_at(local) {
            self.scene.begin();
            self.drag = Some(Drag::Resize {
                id,
                handle,
                origin,
                press: at,
            });
            cx.notify();
            return;
        }
        if let Some(ix) = self.node_at(at) {
            let id = self.scene.canvas.nodes[ix].id.clone();
            let is_text = self.scene.canvas.nodes[ix].kind == NodeType::Text;
            if self.editing.as_deref().is_some_and(|e| e != id) {
                self.editing = None;
            }
            if event.click_count == 2 && is_text {
                // The text editing hook: see the field.
                self.scene.selection = silica_canvas::Selection::only_node(&id);
                self.editing = Some(id);
                cx.notify();
                return;
            }
            if shift {
                self.scene.selection.toggle_node(&id);
                cx.notify();
                return;
            }
            if !self.scene.selection.has_node(&id) {
                self.scene.selection = silica_canvas::Selection::only_node(&id);
            }
            self.scene.begin();
            self.drag = Some(Drag::Move {
                press: at,
                applied: CPoint::default(),
                primary: id,
                moved: false,
            });
            cx.notify();
            return;
        }
        if let Some(ix) = hit_edge(&self.scene.canvas, at, EDGE_TOLERANCE / self.scale()) {
            let id = self.scene.canvas.edges[ix].id.clone();
            if shift {
                self.scene.selection.toggle_edge(&id);
            } else {
                self.scene.selection = silica_canvas::Selection::only_edge(&id);
            }
            cx.notify();
            return;
        }
        // The background.
        if event.click_count == 2 {
            // A new card is centred on the pointer, unsnapped, and opens
            // for editing (observed).
            let (w, h) = silica_canvas::DEFAULT_CARD;
            self.scene.begin();
            let id = self.scene.add_text_card(
                CPoint::new((at.x - w / 2.0).round(), (at.y - h / 2.0).round()),
                "",
            );
            self.scene.commit();
            self.editing = Some(id);
            self.edited(cx);
            return;
        }
        if !shift {
            self.scene.selection.clear();
            self.editing = None;
        }
        self.drag = Some(Drag::Marquee {
            start: at,
            end: at,
            additive: shift,
        });
        cx.notify();
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let local = self.local(event.position);
        let at = self.to_canvas(event.position);
        let Some(drag) = self.drag.clone() else {
            let over = hit_node(&self.scene.canvas, at)
                .map(|ix| self.scene.canvas.nodes[ix].id.clone())
                .filter(|id| {
                    self.scene
                        .node(id)
                        .is_some_and(|n| n.kind != NodeType::Group)
                });
            if over != self.hovered {
                self.hovered = over;
                cx.notify();
            }
            return;
        };
        match drag {
            Drag::Pan { last } => {
                self.viewport.pan(local.x - last.x, local.y - last.y);
                self.drag = Some(Drag::Pan { last: local });
                cx.emit(CanvasEvent::Viewport(self.view_state_json()));
            }
            Drag::Marquee {
                start, additive, ..
            } => {
                self.drag = Some(Drag::Marquee {
                    start,
                    end: at,
                    additive,
                });
            }
            Drag::Move {
                press,
                applied,
                primary,
                moved,
            } => {
                let total = CPoint::new(at.x - press.x, at.y - press.y);
                let far = total.distance(CPoint::default()) * self.scale() >= DRAG_THRESHOLD;
                if !moved && !far {
                    return;
                }
                // Back to the unsnapped positions plus the new delta, then
                // the snap of the pressed card carries everything with it.
                self.scene
                    .move_selection(total.x - applied.x, total.y - applied.y);
                let mut applied = total;
                if !event.modifiers.alt {
                    if let Some(n) = self.scene.node(&primary) {
                        let correction = CPoint::new(snap(n.x) - n.x, snap(n.y) - n.y);
                        self.scene.move_selection(correction.x, correction.y);
                        applied = CPoint::new(applied.x + correction.x, applied.y + correction.y);
                    }
                }
                self.drag = Some(Drag::Move {
                    press,
                    applied,
                    primary,
                    moved: true,
                });
            }
            Drag::Resize {
                id,
                handle,
                origin,
                press,
            } => {
                let total = CPoint::new(at.x - press.x, at.y - press.y);
                let mut target = handle.resize(&origin, total.x, total.y, MIN_NODE);
                if !event.modifiers.alt {
                    target = snap_rect_sides(handle, &origin, &target);
                }
                if let Some(now) = self.scene.node(&id).map(Rect::of) {
                    let (dx, dy) = resize_delta(handle, &now, &target);
                    self.scene.resize_node(&id, handle, dx, dy);
                }
            }
            Drag::Connect { from, side, .. } => {
                self.drag = Some(Drag::Connect { from, side, to: at });
            }
        }
        cx.notify();
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        let at = self.to_canvas(event.position);
        match drag {
            Drag::Pan { .. } => {}
            Drag::Marquee {
                start,
                end,
                additive,
            } => {
                let rect = Rect::between(start, end);
                let hits: Vec<String> = nodes_in(&self.scene.canvas, &rect)
                    .into_iter()
                    .map(|ix| self.scene.canvas.nodes[ix].id.clone())
                    .collect();
                if !additive {
                    self.scene.selection.clear();
                }
                for id in hits {
                    if !self.scene.selection.has_node(&id) {
                        self.scene.selection.nodes.push(id);
                    }
                }
            }
            Drag::Move { moved, .. } => {
                if moved {
                    self.scene.commit();
                    self.edited(cx);
                    return;
                }
                // A click: `begin` took a snapshot that nothing changed.
                self.scene.commit();
            }
            Drag::Resize { .. } => {
                self.scene.commit();
                self.edited(cx);
                return;
            }
            Drag::Connect { from, side, .. } => {
                let target = hit_node(&self.scene.canvas, at)
                    .map(|ix| &self.scene.canvas.nodes[ix])
                    .filter(|n| n.id != from && n.kind != NodeType::Group)
                    .map(|n| (n.id.clone(), facing_side(&Rect::of(n), at)));
                self.scene.begin();
                let mut created = None;
                let (to, to_side) = match target {
                    Some(found) => found,
                    None => {
                        // Dropped on empty space: Obsidian makes a new card
                        // there, its near side on the drop point, and
                        // connects it. OBSERVE: whether the card opens for
                        // editing at once.
                        let to_side = opposite(side);
                        let (w, h) = silica_canvas::DEFAULT_CARD;
                        let top_left = match to_side {
                            Side::Left => CPoint::new(at.x, at.y - h / 2.0),
                            Side::Right => CPoint::new(at.x - w, at.y - h / 2.0),
                            Side::Top => CPoint::new(at.x - w / 2.0, at.y),
                            Side::Bottom => CPoint::new(at.x - w / 2.0, at.y - h),
                        };
                        let id = self
                            .scene
                            .add_text_card(silica_canvas::snap_point(top_left), "");
                        self.editing = Some(id.clone());
                        created = Some(id.clone());
                        (id, to_side)
                    }
                };
                self.scene.add_edge(&from, side, &to, to_side);
                if let Some(card) = created {
                    // The new card, not its edge, is what the user works
                    // on next.
                    self.scene.selection = silica_canvas::Selection::only_node(&card);
                }
                self.scene.commit();
                self.edited(cx);
                return;
            }
        }
        cx.notify();
    }

    fn scroll(&mut self, event: &ScrollWheelEvent, line_height: Pixels, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(line_height);
        let (mut dx, mut dy) = (f64::from(f32::from(delta.x)), f64::from(f32::from(delta.y)));
        if event.modifiers.control {
            self.viewport
                .zoom_at(dy * WHEEL_ZOOM, self.local(event.position), self.size());
        } else {
            if event.modifiers.shift && dx == 0.0 {
                std::mem::swap(&mut dx, &mut dy);
            }
            self.viewport.pan(dx, dy);
        }
        self.viewport_changed(cx);
    }

    fn pinch(&mut self, event: &PinchEvent, cx: &mut Context<Self>) {
        // The platform's delta is a fraction of the scale; the viewport
        // wants log2 of a factor.
        let factor = f64::from(1.0 + event.delta).max(f64::EPSILON);
        self.viewport
            .zoom_at(factor.log2(), self.local(event.position), self.size());
        self.viewport_changed(cx);
    }

    /// A click on one of the colour swatches above the selection.
    fn set_color(&mut self, color: Color, cx: &mut Context<Self>) {
        self.scene.begin();
        self.scene.set_selection_color(color);
        self.scene.commit();
        self.edited(cx);
    }

    // ---- painting ----

    /// Lays out the content of every visible card for this frame and inserts
    /// the hitbox the mouse listeners check.
    fn prepaint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) -> Prepared {
        let had_size = self.bounds.size.width > px(0.) && self.bounds.size.height > px(0.);
        self.bounds = bounds;
        if !self.framed && (had_size || bounds.size.width > px(0.)) {
            self.fit_all();
            self.announce_viewport = true;
        }
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let mut contents = Vec::new();
        let (Some(state), None) = (self.state.clone(), &self.error) else {
            return Prepared { hitbox, contents };
        };
        let theme = cx.global::<Theme>().clone();
        let scale = self.scale() as f32;
        let visible = self.viewport.visible(self.size());
        for ix in visible_nodes(&self.scene.canvas, &visible) {
            let node = &self.scene.canvas.nodes[ix];
            // A group has content only when it has a background image.
            if node.kind == NodeType::Group && node.background.is_none() {
                continue;
            }
            let frame = self.frame_bounds(node);
            // The content sits inside the border.
            let inner = Bounds::new(
                point(
                    frame.origin.x + px(BORDER_WIDTH),
                    frame.origin.y + px(BORDER_WIDTH),
                ),
                size(
                    (frame.size.width - px(2.0 * BORDER_WIDTH)).max(px(0.)),
                    (frame.size.height - px(2.0 * BORDER_WIDTH)).max(px(0.)),
                ),
            );
            let mut element = match &self.card_editor {
                Some(card) if card.id == node.id => card.editor.clone().into_any_element(),
                _ => self
                    .cards
                    .render(node, inner.size, scale, &state, &theme, window, cx),
            };
            element.layout_as_root(
                size(
                    AvailableSpace::Definite(inner.size.width),
                    AvailableSpace::Definite(inner.size.height),
                ),
                window,
                cx,
            );
            element.prepaint_at(inner.origin, window, cx);
            contents.push(CardContent {
                ix,
                frame: inner,
                element,
            });
        }
        Prepared { hitbox, contents }
    }

    /// A node's frame in window pixels.
    fn frame_bounds(&self, node: &Node) -> Bounds<Pixels> {
        let r = self.screen_rect(&Rect::of(node));
        Bounds::new(
            point(
                self.bounds.origin.x + px(r.x as f32),
                self.bounds.origin.y + px(r.y as f32),
            ),
            size(px(r.width as f32), px(r.height as f32)),
        )
    }

    /// A canvas point in window pixels.
    fn window_point(&self, p: CPoint) -> gpui::Point<Pixels> {
        let s = self.viewport.to_screen(p, self.size());
        point(
            self.bounds.origin.x + px(s.x as f32),
            self.bounds.origin.y + px(s.y as f32),
        )
    }

    fn paint(
        &mut self,
        bounds: Bounds<Pixels>,
        prepared: Prepared,
        window: &mut Window,
        cx: &mut App,
    ) {
        let theme = cx.global::<Theme>().clone();
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            self.paint_grid(bounds, &theme, window);
            if self.error.is_none() {
                let mut contents: std::collections::HashMap<usize, CardContent> =
                    prepared.contents.into_iter().map(|c| (c.ix, c)).collect();
                self.paint_groups(&mut contents, &theme, window, cx);
                self.paint_edges(&theme, window);
                self.paint_cards(&mut contents, &theme, window, cx);
                // Edge labels sit above the cards, as Obsidian's do.
                self.paint_edge_labels(&theme, window, cx);
                self.paint_selection(&theme, window);
                self.paint_overlays(&theme, window);
            }
        });
        let cursor = match (&self.drag, self.space_held) {
            (Some(Drag::Pan { .. }), _) => CursorStyle::ClosedHand,
            (None, true) => CursorStyle::OpenHand,
            (Some(Drag::Connect { .. }), _) => CursorStyle::Crosshair,
            _ => CursorStyle::Arrow,
        };
        window.set_cursor_style(cursor, &prepared.hitbox);
    }

    /// The dot grid, anchored to canvas space: the dots sit on multiples of
    /// the snap grid, so a snapped card lands on them.
    fn paint_grid(&self, bounds: Bounds<Pixels>, theme: &Theme, window: &mut Window) {
        let scale = self.scale();
        let mut spacing = silica_canvas::GRID;
        while spacing * scale < MIN_DOT_SPACING {
            spacing *= 2.0;
        }
        let visible = self.viewport.visible(self.size());
        let first_x = (visible.x / spacing).floor() * spacing;
        let first_y = (visible.y / spacing).floor() * spacing;
        let d = px(DOT_RADIUS * 2.0);
        let mut y = first_y;
        while y <= visible.bottom() {
            let mut x = first_x;
            while x <= visible.right() {
                let p = self.window_point(CPoint::new(x, y));
                let mut quad = fill(
                    Bounds::new(
                        point(p.x - px(DOT_RADIUS), p.y - px(DOT_RADIUS)),
                        size(d, d),
                    ),
                    theme.canvas.dot,
                );
                quad.corner_radii = Corners::all(px(DOT_RADIUS));
                window.paint_quad(quad);
                x += spacing;
            }
            y += spacing;
        }
        let _ = bounds;
    }

    fn paint_groups(
        &self,
        contents: &mut std::collections::HashMap<usize, CardContent>,
        theme: &Theme,
        window: &mut Window,
        cx: &mut App,
    ) {
        let visible = self.viewport.visible(self.size());
        let scale = self.scale();
        let ls = self.label_scale();
        self.label_hits.borrow_mut().clear();
        for ix in visible_nodes(&self.scene.canvas, &visible) {
            let node = &self.scene.canvas.nodes[ix];
            if node.kind != NodeType::Group {
                continue;
            }
            let color = Self::color_of(node.color.as_deref(), theme);
            let frame = self.frame_bounds(node);
            // A tinted body inside a bordered frame, like a card's.
            let mut tint = color.unwrap_or(theme.canvas.card_border);
            tint.a = theme.canvas.tint_alpha;
            let mut quad = fill(frame, tint);
            quad.corner_radii = Corners::all(px((CARD_RADIUS * scale) as f32));
            quad.border_widths = Edges::all(self.border_px());
            quad.border_color = self.border_color(node, theme);
            window.paint_quad(quad);
            if let Some(mut content) = contents.remove(&ix) {
                window.with_content_mask(
                    Some(gpui::ContentMask {
                        bounds: content.frame,
                    }),
                    |window| content.element.paint(window, cx),
                );
            }
            if let Some(label) = node.label.as_deref().filter(|l| !l.is_empty()) {
                let font = GROUP_LABEL_SIZE * ls;
                if font < MIN_LABEL_PX {
                    continue;
                }
                // The pill: solid colour with contrasting text for a coloured
                // group, the default colour washed under muted text for a
                // plain one.
                let (bg, fg) = match color {
                    Some(c) => (c, theme.canvas.on_color),
                    None => {
                        let mut wash = theme.canvas.card_border;
                        wash.a = theme.canvas.wash_alpha;
                        (wash, theme.text_muted)
                    }
                };
                let pad = (
                    px((GROUP_LABEL_PADDING.0 * ls) as f32),
                    px((GROUP_LABEL_PADDING.1 * ls) as f32),
                );
                let max_w = (frame.size.width - pad.0 * 2.0).max(px(MIN_DOT_SPACING as f32));
                let line = shape_label(label, px(font as f32), fg, Some(max_w), window);
                let line_height = px(font as f32);
                let w = line.width + pad.0 * 2.0;
                let h = line_height + pad.1 * 2.0;
                let origin = point(
                    frame.origin.x,
                    frame.origin.y - h - px((LABEL_GAP * ls) as f32),
                );
                let bounds = Bounds::new(origin, size(w, h));
                let mut quad = fill(bounds, bg);
                quad.corner_radii = Corners::all(px((LABEL_RADIUS * ls) as f32));
                window.paint_quad(quad);
                let _ = line.paint(
                    point(origin.x + pad.0, origin.y + pad.1),
                    line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
                self.label_hits.borrow_mut().push((node.id.clone(), bounds));
            }
        }
    }

    fn paint_edges(&self, theme: &Theme, window: &mut Window) {
        let ls = self.label_scale();
        let visible = self.viewport.visible(self.size());
        let width = EDGE_WIDTH * ls as f32;
        for edge in &self.scene.canvas.edges {
            let Some((path, from, to)) = self.edge_geometry(edge) else {
                continue;
            };
            if !path.bounds().inflate(ARROW).intersects(&visible) {
                continue;
            }
            let selected = self.scene.selection.has_edge(&edge.id);
            let color =
                Self::color_of(edge.color.as_deref(), theme).unwrap_or(theme.canvas.card_border);
            if selected {
                // Obsidian keeps the stroke as it is and shows the floating
                // menu; a wider translucent halo makes the pick visible here.
                let mut halo = color;
                halo.a = theme.canvas.wash_alpha * 2.0;
                self.stroke_edge(&path, width * 4.0, halo, window);
            }
            self.stroke_edge(&path, width, color, window);
            let (from_arrow, to_arrow) = edge_ends(edge);
            self.paint_end(from.0, from.1, from_arrow, width, color, window);
            self.paint_end(to.0, to.1, to_arrow, width, color, window);
        }
        // The edge being drawn.
        if let Some(Drag::Connect { from, side, to }) = &self.drag {
            if let Some(n) = self.scene.node(from) {
                let a = anchor(&Rect::of(n), *side);
                let to_side = opposite(*side);
                let path = edge_path_points(a, *side, *to, to_side);
                let color = theme.canvas.card_border;
                self.stroke_edge(&path, width, color, window);
                self.paint_end(a, *side, false, width, color, window);
                let [t, l, r] = arrow_head(*to, path.tangent_at(1.0), ARROW);
                self.fill_triangle(t, l, r, color, window);
            }
        }
    }

    /// The labels of the visible edges, on top of the cards.
    fn paint_edge_labels(&self, theme: &Theme, window: &mut Window, cx: &mut App) {
        let font = EDGE_LABEL_SIZE * self.label_scale();
        if font < MIN_LABEL_PX {
            return;
        }
        let visible = self.viewport.visible(self.size());
        for edge in &self.scene.canvas.edges {
            let Some(label) = edge.label.as_deref().filter(|l| !l.is_empty()) else {
                continue;
            };
            let Some((path, _, _)) = self.edge_geometry(edge) else {
                continue;
            };
            if !path.bounds().inflate(ARROW).intersects(&visible) {
                continue;
            }
            self.paint_pill(label, path.midpoint(), theme, window, cx);
        }
    }

    /// An edge's curve and its two anchors (point and side) on the cards.
    fn edge_geometry(&self, edge: &Edge) -> Option<EdgeGeometry> {
        let from = self.scene.node(&edge.from_node)?;
        let to = self.scene.node(&edge.to_node)?;
        let (fr, tr) = (Rect::of(from), Rect::of(to));
        let from_side = edge
            .from_side
            .unwrap_or_else(|| facing_side(&fr, tr.center()));
        let to_side = edge
            .to_side
            .unwrap_or_else(|| facing_side(&tr, fr.center()));
        let path = silica_canvas::edge_path(&fr, from_side, &tr, to_side);
        Some((
            path,
            (anchor(&fr, from_side), from_side),
            (anchor(&tr, to_side), to_side),
        ))
    }

    /// The end of an edge at a card: an arrow whose tip touches the card,
    /// pointing in, or the straight stub joining the curve to the card.
    fn paint_end(
        &self,
        at: CPoint,
        side: Side,
        arrow: bool,
        width: f32,
        color: Hsla,
        window: &mut Window,
    ) {
        if arrow {
            let n = side_normal(side);
            let [t, l, r] = arrow_head(at, CPoint::new(-n.x, -n.y), ARROW);
            self.fill_triangle(t, l, r, color, window);
        } else {
            let (a, b) = edge_stub(at, side);
            let mut builder = PathBuilder::stroke(px(width));
            builder.move_to(self.window_point(a));
            builder.line_to(self.window_point(b));
            if let Ok(built) = builder.build() {
                window.paint_path(built, color);
            }
        }
    }

    fn stroke_edge(
        &self,
        path: &silica_canvas::EdgePath,
        width: f32,
        color: Hsla,
        window: &mut Window,
    ) {
        let mut builder = PathBuilder::stroke(px(width));
        builder.move_to(self.window_point(path.from));
        builder.cubic_bezier_to(
            self.window_point(path.to),
            self.window_point(path.c1),
            self.window_point(path.c2),
        );
        if let Ok(built) = builder.build() {
            window.paint_path(built, color);
        }
    }

    fn fill_triangle(&self, a: CPoint, b: CPoint, c: CPoint, color: Hsla, window: &mut Window) {
        let mut builder = PathBuilder::fill();
        builder.move_to(self.window_point(a));
        builder.line_to(self.window_point(b));
        builder.line_to(self.window_point(c));
        builder.close();
        if let Ok(built) = builder.build() {
            window.paint_path(built, color);
        }
    }

    /// An edge label: text on a pill of the canvas background, centred on
    /// `at`; 20 units of text in 26 of line, padded 6, radius 4, all
    /// counter-scaled.
    fn paint_pill(&self, text: &str, at: CPoint, theme: &Theme, window: &mut Window, cx: &mut App) {
        let ls = self.label_scale();
        let font = px((EDGE_LABEL_SIZE * ls) as f32);
        let line = shape_label(text, font, theme.text, None, window);
        let line_height = px((EDGE_LABEL_LINE * ls) as f32);
        let pad = px((EDGE_LABEL_PADDING * ls) as f32);
        let w = line.width + pad * 2.0;
        let h = line_height + pad * 2.0;
        let centre = self.window_point(at);
        let origin = point(centre.x - w / 2.0, centre.y - h / 2.0);
        let mut quad = fill(Bounds::new(origin, size(w, h)), theme.canvas.background);
        quad.corner_radii = Corners::all(px((LABEL_RADIUS * ls) as f32));
        window.paint_quad(quad);
        let _ = line.paint(
            point(origin.x + pad, origin.y + pad),
            line_height,
            TextAlign::Left,
            None,
            window,
            cx,
        );
    }

    fn paint_cards(
        &self,
        contents: &mut std::collections::HashMap<usize, CardContent>,
        theme: &Theme,
        window: &mut Window,
        cx: &mut App,
    ) {
        let scale = self.scale();
        let ls = self.label_scale();
        let visible = self.viewport.visible(self.size());
        for ix in visible_nodes(&self.scene.canvas, &visible) {
            let node = &self.scene.canvas.nodes[ix];
            if node.kind == NodeType::Group {
                continue;
            }
            let frame = self.frame_bounds(node);
            // The canvas ground under a tint of the card's colour, inside a
            // border of that colour at 0.7 (the default colour is solid).
            let color = Self::color_of(node.color.as_deref(), theme);
            let mut quad = fill(frame, theme.canvas.background);
            quad.corner_radii = Corners::all(px((CARD_RADIUS * scale) as f32));
            quad.border_widths = Edges::all(self.border_px());
            quad.border_color = self.border_color(node, theme);
            window.paint_quad(quad);
            if let Some(mut tint) = color {
                tint.a = theme.canvas.tint_alpha;
                let mut quad = fill(frame, tint);
                quad.corner_radii = Corners::all(px((CARD_RADIUS * scale) as f32));
                window.paint_quad(quad);
            }
            if let Some(mut content) = contents.remove(&ix) {
                window.with_content_mask(
                    Some(gpui::ContentMask {
                        bounds: content.frame,
                    }),
                    |window| content.element.paint(window, cx),
                );
            }
            // The name above a file card, the address above a link card
            // (Obsidian shows the page's title there, which it fetched).
            let label = match node.kind {
                NodeType::File => node.file.as_deref().map(|f| {
                    crate::explorer::display_name(f.rsplit('/').next().unwrap_or(f)).to_string()
                }),
                NodeType::Link => node.url.clone(),
                _ => None,
            };
            if let Some(label) = label.filter(|l| !l.is_empty()) {
                let font = CARD_LABEL_SIZE * ls;
                if font >= MIN_LABEL_PX {
                    let line_height = font * LABEL_LINE_HEIGHT;
                    let origin = point(
                        frame.origin.x,
                        frame.origin.y - px((line_height + LABEL_GAP * ls) as f32),
                    );
                    paint_label(
                        &label,
                        px(font as f32),
                        theme.canvas.label,
                        origin,
                        Some(frame.size.width),
                        window,
                        cx,
                    );
                }
            }
        }
    }

    /// The ring around each selected card: its own colour, solid, two units
    /// outside the border (Obsidian's `is-focused` shadow); the accent for an
    /// uncoloured card. The resize handles are invisible strips, as
    /// Obsidian's are.
    fn paint_selection(&self, theme: &Theme, window: &mut Window) {
        let scale = self.scale();
        let ring = px((SELECTION_RING * scale).max(1.5) as f32);
        for id in &self.scene.selection.nodes {
            let Some(node) = self.scene.node(id) else {
                continue;
            };
            let frame = self.frame_bounds(node);
            let outline = Bounds::new(
                point(frame.origin.x - ring, frame.origin.y - ring),
                size(
                    frame.size.width + ring * 2.0,
                    frame.size.height + ring * 2.0,
                ),
            );
            let color = Self::color_of(node.color.as_deref(), theme).unwrap_or(theme.accent);
            let mut quad = fill(outline, gpui::transparent_black());
            quad.corner_radii = Corners::all(px((CARD_RADIUS * scale) as f32) + ring);
            quad.border_widths = Edges::all(ring);
            quad.border_color = color;
            window.paint_quad(quad);
        }
    }

    /// The connector dots of the hovered card and the marquee.
    fn paint_overlays(&self, theme: &Theme, window: &mut Window) {
        if self.drag.is_none() || matches!(self.drag, Some(Drag::Connect { .. })) {
            if let Some(node) = self.hovered.as_ref().and_then(|id| self.scene.node(id)) {
                let rect = self.screen_rect(&Rect::of(node));
                for side in [Side::Top, Side::Right, Side::Bottom, Side::Left] {
                    let c = anchor(&rect, side);
                    let p = point(
                        self.bounds.origin.x + px(c.x as f32),
                        self.bounds.origin.y + px(c.y as f32),
                    );
                    let r = px(CONNECTOR_RADIUS);
                    let color = Self::color_of(node.color.as_deref(), theme)
                        .unwrap_or(theme.canvas.card_border);
                    let mut quad = fill(
                        Bounds::new(point(p.x - r, p.y - r), size(r * 2.0, r * 2.0)),
                        color,
                    );
                    quad.corner_radii = Corners::all(r);
                    quad.border_widths = Edges::all(px(1.5));
                    quad.border_color = theme.canvas.background;
                    window.paint_quad(quad);
                }
            }
        }
        if let Some(Drag::Marquee { start, end, .. }) = &self.drag {
            let r = self.screen_rect(&Rect::between(*start, *end));
            let mut wash = theme.accent;
            wash.a = theme.canvas.wash_alpha;
            let mut quad = fill(
                Bounds::new(
                    point(
                        self.bounds.origin.x + px(r.x as f32),
                        self.bounds.origin.y + px(r.y as f32),
                    ),
                    size(px(r.width as f32), px(r.height as f32)),
                ),
                wash,
            );
            quad.border_widths = Edges::all(px(1.0));
            quad.border_color = theme.accent;
            window.paint_quad(quad);
        }
    }

    /// One control group of the top-right stack.
    fn control_group(
        &self,
        prefix: &'static str,
        buttons: &[(&'static str, &'static str, bool)],
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let mut group = div()
            .flex()
            .flex_col()
            .rounded(px(4.))
            .overflow_hidden()
            .bg(theme.canvas.control_group)
            .border_1()
            .border_color(theme.canvas.control_border)
            .occlude();
        for (ix, (id, glyph, enabled)) in buttons.iter().enumerate() {
            let id: &'static str = id;
            let fg = if *enabled {
                theme.canvas.control_fg
            } else {
                theme.text_faint
            };
            group = group.child(
                div()
                    .id((prefix, ix))
                    .w(px(CONTROL_BUTTON))
                    .h(px(CONTROL_BUTTON))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme.canvas.control_bg)
                    .when(*enabled, |d| {
                        d.cursor_pointer()
                            .hover(|s| s.bg(theme.canvas.control_group))
                    })
                    .tooltip(menu::tooltip(control_label(id)))
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.control_pick(id, window, cx)),
                    )
                    .child(icon(glyph, 16.0, fg)),
            );
        }
        group
    }

    /// What a control button does.
    fn control_pick(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        match id {
            "zoom-in" => self.zoom_in(&ZoomIn, window, cx),
            "zoom-out" => self.zoom_out(&ZoomOut, window, cx),
            "reset" => self.reset_zoom(&ResetZoom, window, cx),
            "fit" => self.zoom_to_fit(&ZoomToFit, window, cx),
            "undo" => self.undo(&Undo, window, cx),
            "redo" => self.redo(&Redo, window, cx),
            _ => {}
        }
    }

    /// The control stack in the top-right corner, in Obsidian's four groups:
    /// settings; zoom in, reset, fit, out; undo, redo; help. Settings and
    /// help do nothing yet (the first opens Obsidian's snapping menu, the
    /// second its help page).
    fn render_controls(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .absolute()
            .top(px(CONTROL_MARGIN))
            .right(px(CONTROL_MARGIN))
            .flex()
            .flex_col()
            .gap(px(CONTROL_GAP))
            .child(self.control_group(
                "canvas-settings",
                &[("settings", "settings", false)],
                theme,
                cx,
            ))
            .child(self.control_group(
                "canvas-zoom",
                &[
                    ("zoom-in", "plus", true),
                    ("reset", "rotate-ccw", true),
                    ("fit", "maximize", true),
                    ("zoom-out", "minus", true),
                ],
                theme,
                cx,
            ))
            .child(self.control_group(
                "canvas-history",
                &[
                    ("undo", "undo-2", self.scene.can_undo()),
                    ("redo", "redo-2", self.scene.can_redo()),
                ],
                theme,
                cx,
            ))
            .child(self.control_group("canvas-help", &[("help", "circle-help", false)], theme, cx))
            .into_any_element()
    }

    /// The card menu at the bottom centre: Obsidian's three draggable
    /// buttons. A click adds a card at the middle of the view, or opens the
    /// note or media picker for one (Obsidian also lets them be dragged
    /// onto the canvas).
    fn render_card_menu(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let button = |ix: usize,
                      glyph: &'static str,
                      label: &'static str,
                      enabled: bool,
                      theme: &Theme,
                      cx: &mut Context<Self>| {
            div()
                .id(("canvas-card-menu", ix))
                .w(px(CARD_MENU_BUTTON))
                .h(px(CARD_MENU_BUTTON))
                .flex()
                .items_center()
                .justify_center()
                .when(enabled, |d| {
                    d.cursor_pointer()
                        .hover(|s| s.bg(theme.canvas.control_group))
                })
                .tooltip(menu::tooltip(label))
                .on_click(cx.listener(move |this, _, _, cx| match ix {
                    0 => this.add_card_at_center(cx),
                    1 => cx.emit(CanvasEvent::PickFile(FilePick::Note)),
                    _ => cx.emit(CanvasEvent::PickFile(FilePick::Media)),
                }))
                .child(icon(
                    glyph,
                    24.0,
                    if enabled {
                        theme.canvas.control_fg
                    } else {
                        theme.text_faint
                    },
                ))
        };
        div()
            .absolute()
            .bottom(px(CARD_MENU_BOTTOM))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .rounded(px(4.))
                    .overflow_hidden()
                    .bg(theme.canvas.card_menu_bg)
                    .shadow_md()
                    .occlude()
                    .child(button(
                        0,
                        "sticky-note",
                        "Drag to add card",
                        true,
                        theme,
                        cx,
                    ))
                    .child(button(
                        1,
                        "file-text",
                        "Drag to add note from vault",
                        true,
                        theme,
                        cx,
                    ))
                    .child(button(
                        2,
                        "image",
                        "Drag to add media from vault",
                        true,
                        theme,
                        cx,
                    )),
            )
            .into_any_element()
    }

    /// The icon row floating above the selection, with Obsidian's items for
    /// what is selected, and the colour swatches under it while "Set color"
    /// is open. Hidden while a drag is under way.
    fn render_floating_menu(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.drag.is_some() || self.scene.selection.is_empty() || self.error.is_some() {
            return None;
        }
        const REMOVE: IconItem = IconItem {
            id: "remove",
            icon: "trash-2",
            label: "Remove",
        };
        const COLOR: IconItem = IconItem {
            id: "color",
            icon: "palette",
            label: "Set color",
        };
        const ZOOM: IconItem = IconItem {
            id: "zoom",
            icon: "maximize",
            label: "Zoom to selection (Shift + 2)",
        };
        const EDIT: IconItem = IconItem {
            id: "edit",
            icon: "pencil",
            label: "Edit",
        };
        const DIRECTION: IconItem = IconItem {
            id: "direction",
            icon: "arrow-left-right",
            label: "Line direction",
        };
        const REMOVE_LABEL: IconItem = IconItem {
            id: "remove-label",
            icon: "type",
            label: "Remove label",
        };
        const GROUP: IconItem = IconItem {
            id: "group",
            icon: "layout-template",
            label: "Create group",
        };
        let nodes = &self.scene.selection.nodes;
        let edges = &self.scene.selection.edges;
        let mut items = vec![REMOVE, COLOR, ZOOM];
        let anchor_rect = if nodes.is_empty() && edges.len() == 1 {
            let edge = self.scene.edge(&edges[0])?;
            items.push(DIRECTION);
            if edge.label.as_deref().is_some_and(|l| !l.is_empty()) {
                items.push(REMOVE_LABEL);
            }
            let (path, _, _) = self.edge_geometry(edge)?;
            let m = path.midpoint();
            Rect::new(m.x, m.y, 0.0, 0.0)
        } else if nodes.len() == 1 && edges.is_empty() {
            let node = self.scene.node(&nodes[0])?;
            if matches!(node.kind, NodeType::Text) {
                items.push(EDIT);
            }
            if node.kind == NodeType::File
                && node.file.as_deref().is_some_and(|f| {
                    silica_formats::FileKind::from_path(f) == silica_formats::FileKind::Markdown
                })
            {
                items.push(EDIT);
            }
            Rect::of(node)
        } else {
            if nodes.len() > 1 {
                items.push(GROUP);
            }
            self.scene.selection_bounds()?
        };
        let r = self.screen_rect(&anchor_rect);
        let width = menu::icon_row_width(items.len());
        let height = menu::icon_row_height();
        let view_w = f32::from(self.bounds.size.width);
        let left =
            ((r.x + r.width / 2.0) as f32 - width / 2.0).clamp(0.0, (view_w - width).max(0.0));
        let top = (r.y as f32 - MENU_RISE - height).max(0.0);
        let mut column = div()
            .absolute()
            .left(px(left))
            .top(px(top))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(PALETTE_GAP))
            .child(menu::icon_row("canvas-menu", &items, theme, cx));
        if self.palette_open {
            let swatch = |ix: usize,
                          color: Color,
                          fill_color: Hsla,
                          theme: &Theme,
                          cx: &mut Context<Self>| {
                div()
                    .id(("canvas-swatch", ix))
                    .w(px(SWATCH))
                    .h(px(SWATCH))
                    .rounded_full()
                    .cursor_pointer()
                    .bg(fill_color)
                    .border_1()
                    .border_color(theme.border)
                    .hover(|s| s.border_color(theme.text))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.palette_open = false;
                        this.set_color(color, cx)
                    }))
            };
            let mut row = div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(PALETTE_GAP))
                .p(px(PALETTE_GAP))
                .rounded(px(4.))
                .bg(theme.canvas.background)
                .border_1()
                .border_color(theme.canvas.control_border)
                .shadow_md()
                .occlude()
                .child(swatch(
                    0,
                    Color::Default,
                    theme.canvas.card_border,
                    theme,
                    cx,
                ));
            for n in 1..=6u8 {
                let c = theme.canvas.preset(n).unwrap_or(theme.canvas.card_border);
                row = row.child(swatch(n as usize, Color::Preset(n), c, theme, cx));
            }
            column = column.child(row);
        }
        Some(column.into_any_element())
    }

    /// The right-click menu for what was clicked, with Obsidian's entries
    /// that Silica can carry out.
    fn render_context_menu(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (position, target) = self.context_menu?;
        const SEP: ListItem = ListItem {
            id: "",
            label: menu::SEPARATOR,
        };
        let items: &[ListItem] = match target {
            ContextTarget::Card => &[
                ListItem {
                    id: "zoom",
                    label: "Zoom to selection",
                },
                ListItem {
                    id: "edit",
                    label: "Edit",
                },
                SEP,
                ListItem {
                    id: "cut",
                    label: "Cut",
                },
                ListItem {
                    id: "copy",
                    label: "Copy",
                },
                SEP,
                ListItem {
                    id: "remove",
                    label: "Remove",
                },
            ],
            ContextTarget::Cards => &[
                ListItem {
                    id: "zoom",
                    label: "Zoom to selection",
                },
                ListItem {
                    id: "group",
                    label: "Create group",
                },
                SEP,
                ListItem {
                    id: "cut",
                    label: "Cut",
                },
                ListItem {
                    id: "copy",
                    label: "Copy",
                },
                ListItem {
                    id: "paste",
                    label: "Paste",
                },
                SEP,
                ListItem {
                    id: "remove",
                    label: "Delete",
                },
            ],
            ContextTarget::Edge => &[
                ListItem {
                    id: "follow",
                    label: "Follow connection",
                },
                SEP,
                ListItem {
                    id: "remove",
                    label: "Remove",
                },
            ],
            ContextTarget::Background => &[
                ListItem {
                    id: "add-card",
                    label: "Add card",
                },
                ListItem {
                    id: "add-note",
                    label: "Add note from vault",
                },
                ListItem {
                    id: "add-media",
                    label: "Add media from vault",
                },
                SEP,
                ListItem {
                    id: "undo",
                    label: "Undo",
                },
                SEP,
                ListItem {
                    id: "paste",
                    label: "Paste",
                },
            ],
        };
        Some(menu::context_menu(items, position, theme, cx))
    }

    // ---- persistence ----

    /// Shows `path` with the bytes just read from it. An empty file is an
    /// empty canvas, which is what `canvas:new-file` writes; a file that
    /// does not parse shows its error instead of a canvas and is never
    /// written back. `view_state` is the leaf's record of where the view
    /// was looking; without it the content is framed once the element has
    /// a size.
    pub fn load(
        &mut self,
        path: &str,
        text: &str,
        view_state: Option<Viewport>,
        state: Arc<VaultState>,
        cx: &mut Context<Self>,
    ) {
        if self.path.as_deref() != Some(path) {
            self.editing = None;
            self.drag = None;
            self.hovered = None;
        }
        self.cards.set_canvas_path(Some(path.to_string()));
        self.path = Some(path.to_string());
        self.state = Some(state);
        self.saved_text = text.to_string();
        match parse(text) {
            Ok(canvas) => {
                self.scene = Scene::new(canvas);
                self.error = None;
            }
            Err(err) => {
                self.scene = Scene::new(Canvas::default());
                self.error = Some(err);
            }
        }
        match view_state {
            Some(vp) => {
                self.viewport = Viewport {
                    x: vp.x,
                    y: vp.y,
                    zoom: vp.zoom.clamp(ZOOM_MIN, ZOOM_MAX),
                };
                self.framed = true;
            }
            None => self.framed = false,
        }
        cx.notify();
    }

    /// The file changed on disk under the view, by someone else: the data
    /// is replaced, the selection kept where it still applies, and the
    /// history dropped, since its snapshots describe a file that is gone.
    pub fn reload(&mut self, text: &str, cx: &mut Context<Self>) {
        self.saved_text = text.to_string();
        match parse(text) {
            Ok(canvas) => {
                self.scene.reload(canvas);
                self.error = None;
            }
            Err(err) => {
                self.scene.reload(Canvas::default());
                self.error = Some(err);
            }
        }
        self.cards.clear();
        cx.notify();
    }

    /// Nothing is on screen: the data goes, so a later load of the same
    /// path reads the file afresh.
    pub fn release(&mut self, cx: &mut Context<Self>) {
        self.path = None;
        self.scene = Scene::new(Canvas::default());
        self.saved_text.clear();
        self.error = None;
        self.editing = None;
        self.drag = None;
        self.hovered = None;
        self.cards.set_canvas_path(None);
        cx.notify();
    }

    /// Whether `text`, read from the file, is what the view last read or
    /// wrote: a watcher event carrying it is the view's own save.
    pub fn matches_file(&self, text: &str) -> bool {
        self.saved_text == text
    }

    pub fn is_dirty(&self) -> bool {
        self.scene.is_dirty() && self.error.is_none()
    }

    /// The bytes to write, as Obsidian would write them.
    pub fn to_json(&self) -> String {
        self.scene.to_json()
    }

    /// The save landed: these are the bytes on disk now.
    pub fn mark_saved(&mut self, text: String) {
        self.saved_text = text;
        self.scene.mark_saved();
    }

    /// A vault file changed, was renamed or removed: a card showing it is rebuilt.
    pub fn invalidate(&mut self, path: &str, cx: &mut Context<Self>) {
        self.cards.invalidate(path);
        cx.notify();
    }

    /// The leaf's `viewState` for `workspace.json`: Obsidian's three keys.
    pub fn view_state_json(&self) -> Value {
        json!({
            "x": json_number(self.viewport.x),
            "y": json_number(self.viewport.y),
            "zoom": json_number(self.viewport.zoom),
        })
    }

    /// The viewport a leaf's `viewState` records, `None` when it has none.
    pub fn viewport_from_json(view_state: Option<&Value>) -> Option<Viewport> {
        let v = view_state?;
        Some(Viewport {
            x: v.get("x")?.as_f64()?,
            y: v.get("y")?.as_f64()?,
            zoom: v.get("zoom")?.as_f64()?,
        })
    }

    /// What the view shows, for the control socket's state dump.
    pub fn describe(&self, cx: &App) -> Value {
        json!({
            "path": self.path,
            "error": self.error,
            "nodes": self.scene.canvas.nodes.len(),
            "edges": self.scene.canvas.edges.len(),
            "viewport": {"x": self.viewport.x, "y": self.viewport.y, "zoom": self.viewport.zoom},
            "scale": self.scale(),
            "size": [f32::from(self.bounds.size.width), f32::from(self.bounds.size.height)],
            "origin": [f32::from(self.bounds.origin.x), f32::from(self.bounds.origin.y)],
            "selected": {
                "nodes": self.scene.selection.nodes,
                "edges": self.scene.selection.edges,
            },
            "hovered": self.hovered,
            "editing": self.editing,
            "editing_text": self.card_editor.as_ref().map(|c| c.editor.read(cx).text().to_string()),
            "dirty": self.is_dirty(),
            "can_undo": self.scene.can_undo(),
            "can_redo": self.scene.can_redo(),
            "drag": self.drag.as_ref().map(|d| match d {
                Drag::Pan { .. } => "pan",
                Drag::Marquee { .. } => "marquee",
                Drag::Move { .. } => "move",
                Drag::Resize { .. } => "resize",
                Drag::Connect { .. } => "connect",
            }),
        })
    }
}

impl Render for CanvasView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_editor(window, cx);
        let theme = cx.global::<Theme>().clone();
        let entity = cx.entity();
        let paint_entity = entity.clone();
        let error = self.error.clone();
        let floating = self.render_floating_menu(&theme, cx);
        let context = self.render_context_menu(&theme, cx);
        let card_menu = (error.is_none()).then(|| self.render_card_menu(&theme, cx));
        div()
            .key_context("CanvasView")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::delete_selection))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::duplicate))
            .on_action(cx.listener(Self::group_selection))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::clear_selection))
            .on_action(cx.listener(Self::zoom_in))
            .on_action(cx.listener(Self::zoom_out))
            .on_action(cx.listener(Self::reset_zoom))
            .on_action(cx.listener(Self::zoom_to_fit))
            .on_action(cx.listener(Self::zoom_to_selection))
            .on_action(cx.listener(Self::nudge_left))
            .on_action(cx.listener(Self::nudge_right))
            .on_action(cx.listener(Self::nudge_up))
            .on_action(cx.listener(Self::nudge_down))
            .on_action(cx.listener(Self::nudge_left_far))
            .on_action(cx.listener(Self::nudge_right_far))
            .on_action(cx.listener(Self::nudge_up_far))
            .on_action(cx.listener(Self::nudge_down_far))
            .on_action(cx.listener(Self::edit_selected))
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(theme.bg)
            .child(
                canvas(
                    move |bounds, window, cx| {
                        entity.update(cx, |view, cx| view.prepaint(bounds, window, cx))
                    },
                    move |bounds, prepared, window, cx| {
                        let hitbox = prepared.hitbox.clone();
                        paint_entity
                            .update(cx, |view, cx| view.paint(bounds, prepared, window, cx));
                        let line_height = window.line_height();
                        // An event sent from inside a frame's paint is
                        // dropped; the fit made in prepaint is reported from
                        // the next frame instead.
                        if paint_entity.read(cx).announce_viewport {
                            let e = paint_entity.clone();
                            window.on_next_frame(move |_, cx| {
                                e.update(cx, |v, cx| {
                                    v.announce_viewport = false;
                                    v.viewport_changed(cx);
                                });
                            });
                        }

                        let e = paint_entity.clone();
                        let hb = hitbox.clone();
                        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                            if phase == DispatchPhase::Bubble && hb.is_hovered(window) {
                                e.update(cx, |v, cx| v.mouse_down(event, window, cx));
                            }
                        });
                        let e = paint_entity.clone();
                        let hb = hitbox.clone();
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                            if phase != DispatchPhase::Bubble {
                                return;
                            }
                            let over = hb.is_hovered(window);
                            e.update(cx, |v, cx| {
                                if over || v.drag.is_some() {
                                    v.mouse_move(event, cx);
                                } else if v.hovered.take().is_some() {
                                    cx.notify();
                                }
                            });
                        });
                        let e = paint_entity.clone();
                        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                            if phase == DispatchPhase::Bubble {
                                e.update(cx, |v, cx| v.mouse_up(event, cx));
                            }
                        });
                        let e = paint_entity.clone();
                        let hb = hitbox.clone();
                        window.on_mouse_event(
                            move |event: &ScrollWheelEvent, phase, window, cx| {
                                if phase == DispatchPhase::Bubble && hb.should_handle_scroll(window)
                                {
                                    e.update(cx, |v, cx| v.scroll(event, line_height, cx));
                                }
                            },
                        );
                        let e = paint_entity.clone();
                        let hb = hitbox.clone();
                        window.on_mouse_event(move |event: &PinchEvent, phase, window, cx| {
                            if phase == DispatchPhase::Bubble && hb.is_hovered(window) {
                                e.update(cx, |v, cx| v.pinch(event, cx));
                            }
                        });
                        // Space held turns a left drag into a pan; the key
                        // events reach here while the view has the focus.
                        let e = paint_entity.clone();
                        window.on_key_event(move |event: &KeyDownEvent, phase, _, cx| {
                            if phase == DispatchPhase::Bubble && event.keystroke.key == "space" {
                                e.update(cx, |v, cx| {
                                    if !v.space_held {
                                        v.space_held = true;
                                        cx.notify();
                                    }
                                });
                            }
                        });
                        let e = paint_entity.clone();
                        window.on_key_event(move |event: &KeyUpEvent, phase, _, cx| {
                            if phase == DispatchPhase::Bubble && event.keystroke.key == "space" {
                                e.update(cx, |v, cx| {
                                    v.space_held = false;
                                    cx.notify();
                                });
                            }
                        });
                    },
                )
                .size_full(),
            )
            .children(error.map(|message| {
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme.text_muted)
                    .child(format!("This canvas could not be read: {message}"))
            }))
            .children(floating)
            .children(card_menu)
            .child(self.render_controls(&theme, cx))
            .children(context)
    }
}

// ---- helpers ------------------------------------------------------------------

/// The tooltips of the control stack, as Obsidian words them.
fn control_label(id: &str) -> &'static str {
    match id {
        "settings" => "Canvas settings",
        "zoom-in" => "Zoom in",
        "reset" => "Reset zoom",
        "fit" => "Zoom to fit (Shift + 1)",
        "zoom-out" => "Zoom out",
        "undo" => "Undo",
        "redo" => "Redo",
        _ => "Canvas help",
    }
}

/// The file's canvas: an empty file is an empty canvas (what `canvas:new-file`
/// writes until something is in it).
fn parse(text: &str) -> Result<Canvas, String> {
    if text.trim().is_empty() {
        return Ok(Canvas::default());
    }
    Canvas::from_json(text).map_err(|e| e.to_string())
}

/// A number the way JavaScript prints it: no decimal point when it is whole.
/// The text with the task marker whose `[` is byte `col` of line `line`
/// flipped the way the editor flips one; `None` when those bytes are not a
/// marker (a card built before its text changed can name a stale place).
fn toggle_task_in(text: &str, line: usize, col: usize) -> Option<String> {
    let mut lines: Vec<&str> = text.split('\n').collect();
    let target = *lines.get(line)?;
    let checked = match target.as_bytes().get(col..col + 3) {
        Some([b'[', status, b']']) => *status != b' ',
        _ => return None,
    };
    let with = if checked { "[ ]" } else { "[x]" };
    let replaced = format!("{}{}{}", &target[..col], with, &target[col + 3..]);
    lines[line] = &replaced;
    Some(lines.join("\n"))
}

/// The side across from `side`: where an edge dropped on empty space
/// meets the card made there.
fn opposite(side: Side) -> Side {
    match side {
        Side::Top => Side::Bottom,
        Side::Bottom => Side::Top,
        Side::Left => Side::Right,
        Side::Right => Side::Left,
    }
}

fn json_number(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        json!(value as i64)
    } else {
        json!(value)
    }
}

/// Snaps the sides a handle moves to the grid, leaving the still sides
/// where `origin` has them. Candidate for `silica-canvas`.
fn snap_rect_sides(handle: Handle, origin: &Rect, target: &Rect) -> Rect {
    let moves_left = matches!(handle, Handle::Left | Handle::TopLeft | Handle::BottomLeft);
    let moves_right = matches!(
        handle,
        Handle::Right | Handle::TopRight | Handle::BottomRight
    );
    let moves_top = matches!(handle, Handle::Top | Handle::TopLeft | Handle::TopRight);
    let moves_bottom = matches!(
        handle,
        Handle::Bottom | Handle::BottomLeft | Handle::BottomRight
    );
    let (mut x, mut right) = (target.x, target.right());
    let (mut y, mut bottom) = (target.y, target.bottom());
    if moves_left {
        x = snap(x).min(origin.right() - MIN_NODE.0);
    }
    if moves_right {
        right = snap(right).max(origin.x + MIN_NODE.0);
    }
    if moves_top {
        y = snap(y).min(origin.bottom() - MIN_NODE.1);
    }
    if moves_bottom {
        bottom = snap(bottom).max(origin.y + MIN_NODE.1);
    }
    Rect::new(x, y, right - x, bottom - y)
}

/// The delta `Handle::resize` needs to take `now` to `target`: the moving
/// side's displacement on each axis. Candidate for `silica-canvas`.
fn resize_delta(handle: Handle, now: &Rect, target: &Rect) -> (f64, f64) {
    let dx = match handle {
        Handle::Left | Handle::TopLeft | Handle::BottomLeft => target.x - now.x,
        Handle::Right | Handle::TopRight | Handle::BottomRight => target.right() - now.right(),
        Handle::Top | Handle::Bottom => 0.0,
    };
    let dy = match handle {
        Handle::Top | Handle::TopLeft | Handle::TopRight => target.y - now.y,
        Handle::Bottom | Handle::BottomLeft | Handle::BottomRight => target.bottom() - now.bottom(),
        Handle::Left | Handle::Right => 0.0,
    };
    (dx, dy)
}

/// Shapes one line of label text in the window's font, cut to `max_width`
/// with an ellipsis when it would not fit.
fn shape_label(
    text: &str,
    font_size: Pixels,
    color: Hsla,
    max_width: Option<Pixels>,
    window: &mut Window,
) -> gpui::ShapedLine {
    let style = window.text_style();
    let shape = |text: SharedString, window: &mut Window| {
        let mut run = style.to_run(text.len());
        run.color = color;
        window
            .text_system()
            .shape_line(text, font_size, &[run], None)
    };
    let text: String = text.lines().next().unwrap_or("").to_string();
    let line = shape(text.clone().into(), window);
    let Some(max) = max_width else {
        return line;
    };
    if line.width <= max || text.is_empty() {
        return line;
    }
    // Proportional cut: the labels are short and the fonts proportional
    // enough that one pass lands within a character or two.
    let share = f32::from(max) / f32::from(line.width).max(1.0);
    let chars = text.chars().count();
    let keep = ((chars as f32 * share) as usize).saturating_sub(1);
    let cut: String = text.chars().take(keep).collect();
    shape(format!("{cut}…").into(), window)
}

/// Paints one line of label text at `origin` (its top-left).
fn paint_label(
    text: &str,
    font_size: Pixels,
    color: Hsla,
    origin: gpui::Point<Pixels>,
    max_width: Option<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let line = shape_label(text, font_size, color, max_width, window);
    let line_height = font_size * LABEL_LINE_HEIGHT as f32;
    let _ = line.paint(origin, line_height, TextAlign::Left, None, window, cx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_task_flips_like_the_editor() {
        assert_eq!(
            toggle_task_in("a\n- [ ] t\n- [x] u", 1, 2).as_deref(),
            Some("a\n- [x] t\n- [x] u")
        );
        assert_eq!(toggle_task_in("- [?] t", 0, 2).as_deref(), Some("- [ ] t"));
        assert_eq!(toggle_task_in("- [ ] t", 0, 1), None);
        assert_eq!(toggle_task_in("- [ ] t", 3, 2), None);
    }

    #[test]
    fn an_empty_file_is_an_empty_canvas() {
        assert!(parse("").unwrap().nodes.is_empty());
        assert!(parse("  \n").unwrap().nodes.is_empty());
        assert!(parse("{").is_err());
    }

    #[test]
    fn view_state_round_trips() {
        let vp =
            CanvasView::viewport_from_json(Some(&json!({"x": 10, "y": -5.5, "zoom": -1}))).unwrap();
        assert_eq!((vp.x, vp.y, vp.zoom), (10.0, -5.5, -1.0));
        assert!(CanvasView::viewport_from_json(Some(&json!({"x": 1}))).is_none());
        assert!(CanvasView::viewport_from_json(None).is_none());
        assert_eq!(json_number(250.0), json!(250));
        assert_eq!(json_number(0.5), json!(0.5));
    }

    #[test]
    fn resizing_snaps_the_moving_sides_only() {
        let origin = Rect::new(0.0, 0.0, 100.0, 100.0);
        let target = Handle::Right.resize(&origin, 13.0, 0.0, MIN_NODE);
        let snapped = snap_rect_sides(Handle::Right, &origin, &target);
        assert_eq!((snapped.x, snapped.right()), (0.0, 120.0));
        let target = Handle::TopLeft.resize(&origin, 7.0, 29.0, MIN_NODE);
        let snapped = snap_rect_sides(Handle::TopLeft, &origin, &target);
        assert_eq!((snapped.x, snapped.y), (0.0, 20.0));
        assert_eq!((snapped.right(), snapped.bottom()), (100.0, 100.0));
        let (dx, dy) = resize_delta(Handle::TopLeft, &origin, &snapped);
        assert_eq!((dx, dy), (0.0, 20.0));
    }
}
