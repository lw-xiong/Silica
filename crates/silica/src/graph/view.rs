//! The graph element: builds nodes and edges from the vault cache, runs the
//! force layout on the background executor, and paints through a canvas
//! with zoom, pan, drag, hover and click.
//!
//! Filters, groups, display and forces come from `graph.json`, and the
//! controls panel writes them back there; the global graph shows every
//! note, the local graph the current note and its neighbors.

use super::layout::{Forces, Layout};
use crate::search::inputs;
use crate::text_input::{InputEvent, TextInput};
use crate::theme::Theme;
use crate::workspace::VaultState;
use gpui::{
    actions, canvas, div, fill, point, prelude::*, px, size, App, Bounds, Context, DispatchPhase,
    Entity, EventEmitter, FocusHandle, Focusable, HitboxBehavior, Hsla, KeyBinding, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Point, ScrollWheelEvent,
    SharedString, Subscription, Task, TextAlign, Window,
};
use serde_json::{json, Value};
use silica_formats::config::{GraphColor, GraphColorGroup, GraphConfig};
use silica_formats::FileKind;
use silica_search::{Search, SearchFile};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How long after the last change the panel waits before writing
/// `graph.json`; Obsidian's writes landed about a third of a second after a
/// click.
const SAVE_DELAY: Duration = Duration::from_millis(300);

/// The colours a new group cycles through; Obsidian offers a picker.
const GROUP_COLORS: &[u32] = &[
    0xe93147, 0xec7500, 0xe0ac00, 0x08b94e, 0x00bfbc, 0x086dff, 0x7852ee, 0xd53984,
];

/// The panel's sections, by the `collapse-*` key each keeps in `graph.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Filters,
    Groups,
    Display,
    Forces,
}

impl Section {
    const ALL: [Section; 4] = [
        Section::Filters,
        Section::Groups,
        Section::Display,
        Section::Forces,
    ];

    fn key(self) -> &'static str {
        match self {
            Section::Filters => "collapse-filter",
            Section::Groups => "collapse-color-groups",
            Section::Display => "collapse-display",
            Section::Forces => "collapse-forces",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Section::Filters => "Filters",
            Section::Groups => "Groups",
            Section::Display => "Display",
            Section::Forces => "Forces",
        }
    }
}

/// The panel's sliders: each owns one `graph.json` number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slider {
    TextFade,
    NodeSize,
    LineSize,
    Center,
    Repel,
    Link,
    LinkDistance,
}

impl Slider {
    fn label(self) -> &'static str {
        match self {
            Slider::TextFade => "Text fade threshold",
            Slider::NodeSize => "Node size",
            Slider::LineSize => "Link thickness",
            Slider::Center => "Center force",
            Slider::Repel => "Repel force",
            Slider::Link => "Link force",
            Slider::LinkDistance => "Link distance",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Slider::TextFade => "textFadeMultiplier",
            Slider::NodeSize => "nodeSizeMultiplier",
            Slider::LineSize => "lineSizeMultiplier",
            Slider::Center => "centerStrength",
            Slider::Repel => "repelStrength",
            Slider::Link => "linkStrength",
            Slider::LinkDistance => "linkDistance",
        }
    }

    /// Minimum, maximum and step, as Obsidian's range inputs have them.
    fn range(self) -> (f32, f32, f32) {
        match self {
            Slider::TextFade => (-3.0, 3.0, 0.1),
            Slider::NodeSize => (0.1, 5.0, 0.1),
            Slider::LineSize => (0.1, 5.0, 0.1),
            Slider::Center => (0.0, 1.0, 0.01),
            Slider::Repel => (0.0, 20.0, 0.1),
            Slider::Link => (0.0, 1.0, 0.01),
            Slider::LinkDistance => (30.0, 500.0, 1.0),
        }
    }

    fn is_force(self) -> bool {
        matches!(
            self,
            Slider::Center | Slider::Repel | Slider::Link | Slider::LinkDistance
        )
    }

    fn get(self, config: &GraphConfig) -> f32 {
        let d = Forces::default();
        match self {
            Slider::TextFade => config.text_fade_multiplier.unwrap_or(0.0),
            Slider::NodeSize => config.node_size_multiplier.unwrap_or(1.0),
            Slider::LineSize => config.line_size_multiplier.unwrap_or(1.0),
            Slider::Center => config.center_strength.unwrap_or(d.center),
            Slider::Repel => config.repel_strength.unwrap_or(d.repel),
            Slider::Link => config.link_strength.unwrap_or(d.link),
            Slider::LinkDistance => config.link_distance.unwrap_or(d.link_distance),
        }
    }

    fn set(self, config: &mut GraphConfig, value: f32) {
        let slot = match self {
            Slider::TextFade => &mut config.text_fade_multiplier,
            Slider::NodeSize => &mut config.node_size_multiplier,
            Slider::LineSize => &mut config.line_size_multiplier,
            Slider::Center => &mut config.center_strength,
            Slider::Repel => &mut config.repel_strength,
            Slider::Link => &mut config.link_strength,
            Slider::LinkDistance => &mut config.link_distance,
        };
        *slot = Some(value);
    }

    /// The value at fraction `t` of the track, snapped to the step and
    /// rounded so that it serializes the way a hand would write it.
    fn value_at(self, t: f32) -> f32 {
        let (min, max, step) = self.range();
        let raw = min + t.clamp(0.0, 1.0) * (max - min);
        let steps = ((raw - min) / step).round();
        let value = (min + steps * step).clamp(min, max);
        (value * 100.0).round() / 100.0
    }

    /// Where on the track the value sits, 0 to 1.
    fn fraction(self, value: f32) -> f32 {
        let (min, max, _) = self.range();
        ((value - min) / (max - min)).clamp(0.0, 1.0)
    }
}

/// A number the way JavaScript prints it: no decimal point when it is
/// whole, since Obsidian writes `250` and `1`, not `250.0`.
fn json_number(value: f32) -> Value {
    if value.fract() == 0.0 && value.abs() < 1e9 {
        json!(value as i64)
    } else {
        json!(value)
    }
}

/// Every number under `value`, the groups' colours included, written the
/// way [`json_number`] writes one.
fn integerize(value: &mut Value) {
    match value {
        Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                *value = json_number(f as f32);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(integerize),
        Value::Object(fields) => fields.values_mut().for_each(integerize),
        _ => {}
    }
}

/// `graph.json` as Obsidian creates it: every key, in its order, with the
/// current values. Used when the file does not exist yet, since a file
/// written from the struct alone would be sparse and ordered differently.
pub(crate) fn full_document(config: &GraphConfig) -> Value {
    let collapsed = |key: &str| {
        config
            .extra
            .get(key)
            .and_then(Value::as_bool)
            .unwrap_or(true)
    };
    let number = |value: Option<f32>, default: Value| match value {
        Some(v) => json_number(v),
        None => default,
    };
    let mut groups = json!(config.color_groups);
    integerize(&mut groups);
    json!({
        "collapse-filter": collapsed("collapse-filter"),
        "search": config.search.clone().unwrap_or_default(),
        "showTags": config.show_tags.unwrap_or(false),
        "showAttachments": config.show_attachments.unwrap_or(false),
        "hideUnresolved": config.hide_unresolved.unwrap_or(false),
        "showOrphans": config.show_orphans.unwrap_or(true),
        "collapse-color-groups": collapsed("collapse-color-groups"),
        "colorGroups": groups,
        "collapse-display": collapsed("collapse-display"),
        "showArrow": config.show_arrow.unwrap_or(false),
        "textFadeMultiplier": number(config.text_fade_multiplier, json!(0)),
        "nodeSizeMultiplier": number(config.node_size_multiplier, json!(1)),
        "lineSizeMultiplier": number(config.line_size_multiplier, json!(1)),
        "collapse-forces": collapsed("collapse-forces"),
        "centerStrength": number(config.center_strength, json!(0.518713248970312)),
        "repelStrength": number(config.repel_strength, json!(10)),
        "linkStrength": number(config.link_strength, json!(1)),
        "linkDistance": number(config.link_distance, json!(250)),
        "scale": number(config.scale, json!(1)),
        "close": config.close.unwrap_or(false),
    })
}

/// The config as a value to merge into an existing `graph.json`. The
/// struct skips an empty `colorGroups`, and a merge keeps a key the new
/// value lacks, so the last group removed has to be written as `[]`.
fn merge_document(config: &GraphConfig) -> Value {
    let mut value = serde_json::to_value(config).unwrap_or(Value::Null);
    if let Some(object) = value.as_object_mut() {
        if config.color_groups.is_empty() {
            object.insert("colorGroups".into(), json!([]));
        }
        if config.search.is_none() {
            object.insert("search".into(), json!(""));
        }
    }
    integerize(&mut value);
    value
}

actions!(graph, [ZoomIn, ZoomOut, ResetView]);

pub fn bind_keys(cx: &mut App) {
    let ctx = Some("GraphView");
    cx.bind_keys([
        KeyBinding::new("ctrl-=", ZoomIn, ctx),
        KeyBinding::new("ctrl--", ZoomOut, ctx),
        KeyBinding::new("ctrl-0", ResetView, ctx),
    ]);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Note,
    Attachment,
    Unresolved,
    Tag,
}

#[derive(Debug, Clone)]
pub struct Node {
    /// Vault path, link text for unresolved targets, `#tag` for tags.
    pub id: String,
    pub label: SharedString,
    pub kind: NodeKind,
    pub degree: usize,
    pub color: Option<Hsla>,
}

/// Nodes and edges ready for layout, built off the UI thread.
struct Built {
    nodes: Vec<Node>,
    edges: Vec<(u32, u32)>,
    neighbors: Vec<Vec<usize>>,
}

pub enum GraphEvent {
    Open(String),
}

struct Drag {
    node: Option<usize>,
    start: Point<Pixels>,
    last: Point<Pixels>,
    moved: bool,
}

pub struct GraphView {
    focus_handle: FocusHandle,
    state: Option<Arc<VaultState>>,
    config: GraphConfig,
    /// The Filters section's search box; its text is `config.search`.
    search: Entity<TextInput>,
    /// One field serves whichever group's query is being edited.
    group_field: Entity<TextInput>,
    editing_group: Option<usize>,
    /// The slider the mouse is holding.
    slider_drag: Option<Slider>,
    /// The debounced `graph.json` write.
    save_task: Option<Task<()>>,
    /// The panel changed something the file does not have yet, so a vault
    /// refresh must not reload the config over it.
    config_dirty: bool,
    /// Pre-compiled color group searches, cached to avoid recompiling 97+
    /// regexes against every node on every rebuild.
    cached_color_groups: Vec<(Arc<Search>, Hsla)>,
    /// The color_groups config that cached_color_groups was built from,
    /// used to detect when recompilation is needed.
    cached_color_config: Vec<GraphColorGroup>,
    /// Local graph depth: how many hops from the centre. 1 = one hop.
    local_depth: usize,
    /// Local graph direction: follow outgoing edges (centre -> neighbours).
    local_forelinks: bool,
    /// Local graph direction: follow incoming edges (neighbours -> centre).
    local_backlinks: bool,
    /// Local graph direction: keep edges between neighbours at the same depth.
    local_interlinks: bool,
    _subscriptions: Vec<Subscription>,
    nodes: Arc<Vec<Node>>,
    edges: Arc<Vec<(u32, u32)>>,
    neighbors: Arc<Vec<Vec<usize>>>,
    index: HashMap<String, usize>,
    layout: Arc<Mutex<Layout>>,
    running: bool,
    scale: f32,
    /// Pan in screen pixels from the element center.
    offset: Point<f32>,
    bounds: Bounds<Pixels>,
    hovered: Option<usize>,
    drag: Option<Drag>,
    /// Local graph centered on this path, else the global graph.
    local: Option<String>,
    current: Option<String>,
    generation: u64,
    building: bool,
    /// The user zoomed or panned; stop auto-fitting the view.
    user_adjusted: bool,
    /// A zoom read from `graph.json`: the auto-fit then only centers the
    /// layout and leaves the scale alone.
    pinned_scale: Option<f32>,
    /// The `graph.json` scale the camera has already acted on, so that a
    /// refresh does not keep reinstating a zoom the user has reset away from.
    adopted_scale: Option<f32>,
    /// The global graph's camera while a local graph is on screen. Obsidian
    /// keeps the two in separate leaves, each with its own zoom, so switching
    /// back has to find the global one where it was left.
    global_camera: Option<(f32, Option<f32>, bool)>,
}

impl EventEmitter<GraphEvent> for GraphView {}

impl Focusable for GraphView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl GraphView {
    pub fn new(cx: &mut Context<Self>) -> GraphView {
        let search = cx.new(|cx| TextInput::new("Search files...", cx));
        let group_field = cx.new(|cx| TextInput::new("Enter query...", cx));
        let subscriptions = vec![
            cx.subscribe(&search, |this, input, event: &InputEvent, cx| {
                if let InputEvent::Changed = event {
                    let text = input.read(cx).text().to_string();
                    if this.config.search.as_deref().unwrap_or("") != text {
                        this.config.search = Some(text);
                        this.rebuild(cx);
                        this.schedule_save(cx);
                    }
                }
            }),
            cx.subscribe(
                &group_field,
                |this, _, event: &InputEvent, cx| match event {
                    InputEvent::Submit => this.commit_group(cx),
                    InputEvent::Cancel => {
                        this.editing_group = None;
                        cx.notify();
                    }
                    _ => {}
                },
            ),
        ];
        GraphView {
            focus_handle: cx.focus_handle(),
            state: None,
            config: GraphConfig::default(),
            search,
            group_field,
            editing_group: None,
            slider_drag: None,
            save_task: None,
            config_dirty: false,
            cached_color_groups: Vec::new(),
            cached_color_config: Vec::new(),
            local_depth: 1,
            local_forelinks: true,
            local_backlinks: true,
            local_interlinks: false,
            _subscriptions: subscriptions,
            nodes: Arc::new(Vec::new()),
            edges: Arc::new(Vec::new()),
            neighbors: Arc::new(Vec::new()),
            index: HashMap::new(),
            layout: Arc::new(Mutex::new(Layout::new(0, Vec::new(), Forces::default()))),
            running: false,
            scale: 1.0,
            offset: point(0.0, 0.0),
            bounds: Bounds::default(),
            hovered: None,
            drag: None,
            local: None,
            current: None,
            generation: 0,
            building: false,
            user_adjusted: false,
            pinned_scale: None,
            adopted_scale: None,
            global_camera: None,
        }
    }

    pub fn set_state(&mut self, state: Arc<VaultState>, cx: &mut Context<Self>) {
        // Only reload config from disk if we don't have unsaved panel changes.
        // If config_dirty is true, the user has edited panel controls that
        // haven't been saved to graph.json yet — keep those in memory.
        if !self.config_dirty {
            self.config = state.vault().graph_config().unwrap_or_default();
        }
        let text = self.config.search.clone().unwrap_or_default();
        self.search.update(cx, |input, cx| {
            if input.text() != text {
                input.set_text(text, cx);
            }
        });
        // A local graph does not start from `graph.json`: against a file
        // holding `scale: 8` the global graph opened at 7.926 and the local
        // one at 1 (see [[Specification#Observed graph.json write behavior]]).
        if !self.is_local()
            && adopts_saved_scale(self.config.scale, self.adopted_scale, self.user_adjusted)
        {
            self.adopted_scale = self.config.scale;
            self.pinned_scale = saved_pin(self.config.scale);
            if let Some(scale) = self.pinned_scale {
                self.scale = scale;
            }
        }
        self.state = Some(state);
        self.rebuild(cx);
    }

    /// The note to highlight, and to center a local graph on.
    pub fn set_current(&mut self, path: Option<String>, cx: &mut Context<Self>) {
        let changed = self.current != path;
        self.current = path.clone();
        if self.local.is_some() && changed {
            self.local = path;
            // A local graph around another note is a new view: frame it.
            self.user_adjusted = false;
            self.rebuild(cx);
        } else if changed {
            cx.notify();
        }
    }

    pub fn set_local(&mut self, local: bool, cx: &mut Context<Self>) {
        let want = if local { self.current.clone() } else { None };
        let was_local = self.is_local();
        // The camera as the outgoing graph leaves it, read before the switch
        // resets the auto-fit.
        let camera = (self.scale, self.pinned_scale, self.user_adjusted);
        if self.local != want || (local && self.current.is_none()) {
            self.local = want;
            self.user_adjusted = false;
            if local {
                // Opened at 1, centred on the note, whatever zoom the global
                // graph was left at and whatever `graph.json` pins. The
                // global camera is put aside rather than thrown away.
                if !was_local {
                    self.global_camera = Some(camera);
                }
                self.pinned_scale = Some(1.0);
                self.scale = 1.0;
            } else if let Some((scale, pinned, adjusted)) = self.global_camera.take() {
                // Back to the global graph, where its own zoom was left.
                self.scale = scale;
                self.pinned_scale = pinned;
                self.user_adjusted = adjusted;
            }
            self.rebuild(cx);
        }
    }

    pub fn is_local(&self) -> bool {
        self.local.is_some()
    }

    /// What the graph is showing, for the control socket's state dump. The
    /// offset is the pan in screen pixels from the element centre, and
    /// `hovered` the label under the pointer, which is the only node the
    /// view names on screen.
    pub fn describe(&self) -> Value {
        json!({
            "local": self.is_local(),
            "nodes": self.nodes.len(),
            "scale": self.scale,
            "offset": [self.offset.x, self.offset.y],
            "hovered": self
                .hovered
                .and_then(|ix| self.nodes.get(ix))
                .map(|node| node.label.clone()),
            "controls": {
                "open": !self.config.close.unwrap_or(false),
                "sections": Section::ALL
                    .iter()
                    .filter(|s| !self.section_collapsed(**s))
                    .map(|s| s.label())
                    .collect::<Vec<_>>(),
                "editing_group": self.editing_group,
                "slider_drag": self.slider_drag.map(Slider::key),
                "unsaved": self.config_dirty,
            },
            "config": serde_json::to_value(&self.config).unwrap_or(Value::Null),
        })
    }

    fn forces(&self) -> Forces {
        let d = Forces::default();
        Forces {
            center: self.config.center_strength.unwrap_or(d.center),
            repel: self.config.repel_strength.unwrap_or(d.repel),
            link: self.config.link_strength.unwrap_or(d.link),
            link_distance: self.config.link_distance.unwrap_or(d.link_distance),
        }
    }

    /// Rebuilds nodes and edges from the cache on the background executor,
    /// then restarts the simulation.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        self.generation += 1;
        let generation = self.generation;
        // Read config from disk via graph.json — this picks up any changes
        // made by save_now() or external edits, so the graph always shows
        // the current graph.json state rather than a stale in-memory copy.
        let config = state.vault().graph_config().unwrap_or_default();
        let local = self.local.clone();
        let local_depth = self.local_depth;
        let local_forelinks = self.local_forelinks;
        let local_backlinks = self.local_backlinks;
        let local_interlinks = self.local_interlinks;

        // Compile color groups only when config.color_groups changes —
        // avoids recompiling 97+ regexes against every node on every rebuild.
        if self.cached_color_config != config.color_groups {
            self.cached_color_groups = config
                .color_groups
                .iter()
                .filter(|g| !g.query.trim().is_empty())
                .filter_map(|g| {
                    let search = Arc::new(Search::new(&g.query).ok()?);
                    let rgb = g.color.rgb;
                    let mut color: Hsla = gpui::rgb(rgb).into();
                    color.a = g.color.a.clamp(0.0, 1.0);
                    Some((search, color))
                })
                .collect();
            self.cached_color_config = config.color_groups.clone();
        }
        let groups = self.cached_color_groups.clone();

        self.building = true;
        cx.spawn(async move |this, cx| {
            let built = cx
                .background_executor()
                .spawn(async move {
                    build(
                        &state,
                        &config,
                        local.as_deref(),
                        local_depth,
                        local_forelinks,
                        local_backlinks,
                        local_interlinks,
                        &groups,
                    )
                })
                .await;
            this.update(cx, |view, cx| {
                if view.generation != generation {
                    return;
                }
                view.building = false;
                view.apply(built, cx);
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn apply(&mut self, built: Built, cx: &mut Context<Self>) {
        // Keep positions of nodes that survive the rebuild.
        let old: HashMap<String, (f32, f32)> = {
            let layout = self.layout.lock().unwrap_or_else(|e| e.into_inner());
            self.nodes
                .iter()
                .enumerate()
                .filter_map(|(i, n)| layout.bodies.get(i).map(|b| (n.id.clone(), (b.x, b.y))))
                .collect()
        };
        let mut layout = Layout::new(built.nodes.len(), built.edges.clone(), self.forces());
        for (i, node) in built.nodes.iter().enumerate() {
            layout.bodies[i].mass = 1.0 + (node.degree as f32).sqrt() * 0.5;
            if let Some((x, y)) = old.get(&node.id) {
                layout.bodies[i].x = *x;
                layout.bodies[i].y = *y;
            }
        }
        self.index = built
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.clone(), i))
            .collect();
        self.nodes = Arc::new(built.nodes);
        self.edges = Arc::new(built.edges);
        self.neighbors = Arc::new(built.neighbors);
        self.hovered = None;
        self.layout = Arc::new(Mutex::new(layout));
        // The loop over the old layout breaks on its next tick, when it sees
        // the swap; clearing the flag here is what lets the new layout get a
        // loop of its own, instead of inheriting a `running` that is already
        // nobody's.
        self.running = false;
        self.start_simulation(cx);
        cx.notify();
    }

    /// Scales and centers the view on the whole layout.
    fn fit(&mut self) {
        let world = self
            .layout
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .bounds();
        let w = f32::from(self.bounds.size.width);
        let h = f32::from(self.bounds.size.height);
        if let Some((scale, offset)) = frame(w, h, world, self.pinned_scale) {
            self.scale = scale;
            self.offset = offset;
        }
    }

    fn start_simulation(&mut self, cx: &mut Context<Self>) {
        if self.running {
            return;
        }
        self.running = true;
        let layout = self.layout.clone();
        cx.spawn(async move |this, cx| {
            loop {
                let shared = layout.clone();
                let settled = cx
                    .background_executor()
                    .spawn(async move {
                        let mut l = shared.lock().unwrap_or_else(|e| e.into_inner());
                        for _ in 0..2 {
                            l.step();
                        }
                        l.is_settled()
                    })
                    .await;
                let alive = this
                    .update(cx, |view, cx| {
                        if !view.user_adjusted && view.drag.is_none() {
                            view.fit();
                        }
                        cx.notify();
                        // A rebuild swapped the layout: this loop is stale.
                        Arc::ptr_eq(&view.layout, &layout)
                    })
                    .unwrap_or(false);
                if !alive {
                    break;
                }
                if settled {
                    this.update(cx, |view, _| view.running = false).ok();
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
            }
        })
        .detach();
    }

    fn reheat(&mut self, cx: &mut Context<Self>) {
        self.layout
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .reheat();
        self.start_simulation(cx);
    }

    // ----- geometry --------------------------------------------------------------

    fn center(&self) -> Point<f32> {
        point(
            f32::from(self.bounds.origin.x + self.bounds.size.width / 2.) + self.offset.x,
            f32::from(self.bounds.origin.y + self.bounds.size.height / 2.) + self.offset.y,
        )
    }

    fn to_screen(&self, x: f32, y: f32) -> Point<Pixels> {
        let c = self.center();
        point(px(c.x + x * self.scale), px(c.y + y * self.scale))
    }

    fn to_world(&self, p: Point<Pixels>) -> (f32, f32) {
        let c = self.center();
        (
            (f32::from(p.x) - c.x) / self.scale,
            (f32::from(p.y) - c.y) / self.scale,
        )
    }

    fn node_radius(&self, node: &Node) -> f32 {
        let mult = self.config.node_size_multiplier.unwrap_or(1.0);
        (2.5 + (node.degree as f32).sqrt() * 1.2) * mult
    }

    /// Zoom level from which labels show: later for bigger graphs, as
    /// `textFadeMultiplier` shifts it.
    fn label_threshold(&self) -> f32 {
        let n = self.nodes.len().max(1) as f32;
        let fade = self.config.text_fade_multiplier.unwrap_or(0.0);
        0.4 + (n / 300.0).ln().max(0.0) * 0.5 + fade * 0.5
    }

    fn hit_test(&self, p: Point<Pixels>) -> Option<usize> {
        let layout = self.layout.lock().unwrap_or_else(|e| e.into_inner());
        let (wx, wy) = self.to_world(p);
        let mut best: Option<(usize, f32)> = None;
        for (i, b) in layout.bodies.iter().enumerate() {
            let r = (self.node_radius(&self.nodes[i]) + 4.0) / self.scale.max(0.01);
            let d2 = (b.x - wx).powi(2) + (b.y - wy).powi(2);
            if d2 <= r * r && best.is_none_or(|(_, bd)| d2 < bd) {
                best = Some((i, d2));
            }
        }
        best.map(|(i, _)| i)
    }

    fn zoom_at(&mut self, factor: f32, at: Point<Pixels>, cx: &mut Context<Self>) {
        self.user_adjusted = true;
        let (wx, wy) = self.to_world(at);
        self.scale = (self.scale * factor).clamp(0.05, 8.0);
        // Keep the world point under the cursor fixed.
        let c = self.center();
        let target = self.to_screen(wx, wy);
        self.offset.x += f32::from(at.x) - f32::from(target.x);
        self.offset.y += f32::from(at.y) - f32::from(target.y);
        let _ = c;
        cx.notify();
    }

    fn zoom_in(&mut self, _: &ZoomIn, _: &mut Window, cx: &mut Context<Self>) {
        let at = self.bounds.center();
        self.zoom_at(1.25, at, cx);
    }

    fn zoom_out(&mut self, _: &ZoomOut, _: &mut Window, cx: &mut Context<Self>) {
        let at = self.bounds.center();
        self.zoom_at(0.8, at, cx);
    }

    fn reset_view(&mut self, _: &ResetView, _: &mut Window, cx: &mut Context<Self>) {
        // Count the reset as a user adjustment so the simulation loop
        // (which calls fit() while !user_adjusted) does not override it.
        self.user_adjusted = true;
        // Fit to the window is what the command promises, so a zoom carried
        // over from graph.json gives way to it. Counting that zoom as adopted
        // is what makes the reset last: the vault refresh behind the next
        // note saved or created would otherwise pin the camera right back.
        self.pinned_scale = None;
        self.adopted_scale = self.config.scale;
        self.fit();
        cx.notify();
    }

    // ----- mouse -----------------------------------------------------------------

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
        let node = self.hit_test(event.position);
        if let Some(ix) = node {
            if let Some(b) = self
                .layout
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .bodies
                .get_mut(ix)
            {
                b.pinned = true;
            }
        }
        self.drag = Some(Drag {
            node,
            start: event.position,
            last: event.position,
            moved: false,
        });
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if let Some(drag) = &mut self.drag {
            let dx = event.position.x - drag.last.x;
            let dy = event.position.y - drag.last.y;
            drag.last = event.position;
            let far = (event.position.x - drag.start.x).abs() > px(3.)
                || (event.position.y - drag.start.y).abs() > px(3.);
            if far {
                drag.moved = true;
            }
            match drag.node {
                Some(ix) => {
                    let (wx, wy) = self.to_world(event.position);
                    if let Some(b) = self
                        .layout
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .bodies
                        .get_mut(ix)
                    {
                        b.x = wx;
                        b.y = wy;
                    }
                    self.reheat(cx);
                }
                None => {
                    self.user_adjusted = true;
                    self.offset.x += f32::from(dx);
                    self.offset.y += f32::from(dy);
                }
            }
            cx.notify();
        } else {
            let hit = self.hit_test(event.position);
            if hit != self.hovered {
                self.hovered = hit;
                cx.notify();
            }
        }
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        if let Some(ix) = drag.node {
            if let Some(b) = self
                .layout
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .bodies
                .get_mut(ix)
            {
                b.pinned = false;
            }
            if !drag.moved && event.button == MouseButton::Left {
                if let Some(node) = self.nodes.get(ix) {
                    if matches!(node.kind, NodeKind::Note | NodeKind::Attachment) {
                        cx.emit(GraphEvent::Open(node.id.clone()));
                    }
                }
            }
        }
        cx.notify();
    }

    fn scroll(&mut self, event: &ScrollWheelEvent, line_height: Pixels, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(line_height);
        self.zoom_at(wheel_zoom_factor(f32::from(delta.y)), event.position, cx);
    }

    fn toggle_local(&mut self, cx: &mut Context<Self>) {
        let local = !self.is_local();
        self.set_local(local, cx);
    }

    // ----- the controls panel ----------------------------------------------------

    fn section_collapsed(&self, section: Section) -> bool {
        self.config
            .extra
            .get(section.key())
            .and_then(Value::as_bool)
            .unwrap_or(true)
    }

    /// A click on a section header: Obsidian writes the `collapse-*` key.
    fn toggle_section(&mut self, section: Section, cx: &mut Context<Self>) {
        let now = !self.section_collapsed(section);
        self.config
            .extra
            .insert(section.key().to_string(), json!(now));
        self.schedule_save(cx);
        cx.notify();
    }

    /// One of the Filters toggles, or Display's Arrows: the key, flipped.
    fn toggle_flag(&mut self, key: &str, cx: &mut Context<Self>) {
        let (slot, default): (&mut Option<bool>, bool) = match key {
            "showTags" => (&mut self.config.show_tags, false),
            "showAttachments" => (&mut self.config.show_attachments, false),
            "hideUnresolved" => (&mut self.config.hide_unresolved, false),
            "showOrphans" => (&mut self.config.show_orphans, true),
            "showArrow" => (&mut self.config.show_arrow, false),
            _ => return,
        };
        let now = !slot.unwrap_or(default);
        *slot = Some(now);
        if key != "showArrow" {
            self.rebuild(cx);
        }
        self.schedule_save(cx);
        cx.notify();
    }

    /// The mouse at `x` on a slider's track.
    fn slide_to(
        &mut self,
        slider: Slider,
        track: Bounds<Pixels>,
        x: Pixels,
        cx: &mut Context<Self>,
    ) {
        let width = f32::from(track.size.width).max(1.0);
        let t = (f32::from(x) - f32::from(track.left())) / width;
        let value = slider.value_at(t);
        if slider.get(&self.config) == value {
            return;
        }
        slider.set(&mut self.config, value);
        self.config_dirty = true;
        if slider.is_force() {
            self.apply_forces(cx);
        }
        cx.notify();
    }

    /// Gives the running layout the forces the config now names.
    fn apply_forces(&mut self, cx: &mut Context<Self>) {
        let forces = self.forces();
        self.layout.lock().unwrap_or_else(|e| e.into_inner()).forces = forces;
        self.reheat(cx);
    }

    fn set_closed(&mut self, closed: bool, cx: &mut Context<Self>) {
        self.config.close = Some(closed);
        self.schedule_save(cx);
        cx.notify();
    }

    /// "Start timelapse animation": the layout again from its first
    /// positions, so the graph can be watched settling.
    fn restart_layout(&mut self, cx: &mut Context<Self>) {
        let mut layout = Layout::new(self.nodes.len(), (*self.edges).clone(), self.forces());
        for (i, node) in self.nodes.iter().enumerate() {
            layout.bodies[i].mass = 1.0 + (node.degree as f32).sqrt() * 0.5;
        }
        self.layout = Arc::new(Mutex::new(layout));
        self.running = false;
        self.user_adjusted = false;
        self.start_simulation(cx);
        cx.notify();
    }

    fn add_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ix = self.config.color_groups.len();
        let rgb = GROUP_COLORS[ix % GROUP_COLORS.len()];
        self.config.color_groups.push(GraphColorGroup {
            query: String::new(),
            color: GraphColor { a: 1.0, rgb },
            extra: Default::default(),
        });
        self.schedule_save(cx);
        self.start_edit_group(ix, window, cx);
    }

    fn remove_group(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.config.color_groups.len() {
            self.config.color_groups.remove(ix);
            self.editing_group = None;
            self.rebuild(cx);
            self.schedule_save(cx);
        }
    }

    /// The swatch: the next colour of the cycle.
    fn cycle_group_color(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(group) = self.config.color_groups.get_mut(ix) else {
            return;
        };
        let at = GROUP_COLORS
            .iter()
            .position(|c| *c == group.color.rgb)
            .map(|p| (p + 1) % GROUP_COLORS.len())
            .unwrap_or(0);
        group.color.rgb = GROUP_COLORS[at];
        self.rebuild(cx);
        self.schedule_save(cx);
    }

    fn start_edit_group(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(query) = self.config.color_groups.get(ix).map(|g| g.query.clone()) else {
            return;
        };
        self.editing_group = Some(ix);
        self.group_field.update(cx, |input, cx| {
            input.set_text(query, cx);
            input.select_all(cx);
        });
        window.focus(&self.group_field.focus_handle(cx), cx);
        cx.notify();
    }

    fn commit_group(&mut self, cx: &mut Context<Self>) {
        let Some(ix) = self.editing_group.take() else {
            return;
        };
        let text = self.group_field.read(cx).text().to_string();
        if let Some(group) = self.config.color_groups.get_mut(ix) {
            if group.query != text {
                group.query = text;
                self.rebuild(cx);
                self.schedule_save(cx);
            }
        }
        cx.notify();
    }

    /// Writes `graph.json` a beat after the last change, so a slider drag
    /// is one write rather than one per pixel.
    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        self.config_dirty = true;
        self.save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            this.update(cx, |view, _| view.save_now()).ok();
        }));
    }

    /// The write itself. A vault that has no `graph.json` gets the whole
    /// document in Obsidian's key order; one that has it gets the changed
    /// keys merged in, so the numbers Obsidian wrote keep their digits.
    fn save_now(&mut self) {
        self.save_task = None;
        let Some(state) = self.state.clone() else {
            return;
        };
        let vault = state.vault();
        let path = vault.absolute(&format!("{}/graph.json", vault.config_dir()));
        let result = if path.is_file() {
            vault.write_config("graph.json", &merge_document(&self.config))
        } else {
            vault.replace_config("graph.json", &full_document(&self.config))
        };
        match result {
            Ok(()) => self.config_dirty = false,
            Err(err) => eprintln!("graph.json could not be written: {err}"),
        }
    }

    // ----- panel elements --------------------------------------------------------

    fn pill(on: bool, theme: &Theme) -> gpui::Div {
        div()
            .relative()
            .flex_shrink_0()
            .w(px(30.))
            .h(px(16.))
            .rounded_full()
            .bg(if on { theme.accent } else { theme.bg_active })
            .child(
                div()
                    .absolute()
                    .top(px(2.))
                    .left(px(if on { 16. } else { 2. }))
                    .w(px(12.))
                    .h(px(12.))
                    .rounded_full()
                    .bg(theme.bg),
            )
    }

    fn flag_row(
        &self,
        label: &'static str,
        key: &'static str,
        on: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(SharedString::from(format!("graph-flag-{key}")))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap_2()
            .px_2()
            .py(px(3.))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_flag(key, cx)))
            .child(div().text_color(theme.text).child(label))
            .child(Self::pill(on, theme))
    }

    fn slider_row(&self, slider: Slider, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let value = slider.get(&self.config);
        let t = slider.fraction(value);
        let entity = cx.entity();
        let (track_color, fill_color, knob_color) = (theme.bg_active, theme.accent, theme.text);
        let track = canvas(
            move |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
            move |bounds, hitbox, window, _| {
                let y = bounds.center().y;
                let line = Bounds::new(
                    point(bounds.left(), y - px(2.)),
                    size(bounds.size.width, px(4.)),
                );
                let mut quad = fill(line, track_color);
                quad.corner_radii = gpui::Corners::all(px(2.));
                window.paint_quad(quad);
                let filled = bounds.size.width * t;
                let mut quad = fill(Bounds::new(line.origin, size(filled, px(4.))), fill_color);
                quad.corner_radii = gpui::Corners::all(px(2.));
                window.paint_quad(quad);
                let knob = Bounds::new(
                    point(bounds.left() + filled - px(6.), y - px(6.)),
                    size(px(12.), px(12.)),
                );
                let mut quad = fill(knob, knob_color);
                quad.corner_radii = gpui::Corners::all(px(6.));
                window.paint_quad(quad);

                let e = entity.clone();
                window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble
                        && event.button == MouseButton::Left
                        && hitbox.is_hovered(window)
                    {
                        cx.stop_propagation();
                        e.update(cx, |view, cx| {
                            view.slider_drag = Some(slider);
                            view.slide_to(slider, bounds, event.position.x, cx);
                        });
                    }
                });
                let e = entity.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble && e.read(cx).slider_drag == Some(slider) {
                        e.update(cx, |view, cx| {
                            view.slide_to(slider, bounds, event.position.x, cx)
                        });
                    }
                });
                let e = entity.clone();
                window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble && e.read(cx).slider_drag == Some(slider) {
                        e.update(cx, |view, cx| {
                            view.slider_drag = None;
                            view.schedule_save(cx);
                        });
                    }
                });
            },
        )
        .w(px(100.))
        .h(px(16.));
        let shown = if slider.range().2 >= 1.0 {
            format!("{value:.0}")
        } else {
            format!("{value:.2}")
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string()
        };
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap_2()
            .px_2()
            .py(px(3.))
            .child(div().text_color(theme.text).child(slider.label()))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .child(track)
                    .child(
                        div()
                            .w(px(30.))
                            .text_right()
                            .text_color(theme.text_muted)
                            .child(SharedString::from(shown)),
                    ),
            )
    }

    fn icon_button(
        name: &'static str,
        glyph: &'static str,
        theme: &Theme,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(name)
            .w(px(22.))
            .h(px(22.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_sm()
            .cursor_pointer()
            .text_size(px(13.))
            .text_color(theme.text_muted)
            .hover(|s| s.bg(theme.bg_alt).text_color(theme.text))
            .child(glyph)
    }

    fn section(
        &self,
        section: Section,
        body: Vec<gpui::AnyElement>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let collapsed = self.section_collapsed(section);
        div()
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(theme.border)
            .child(
                div()
                    .id(SharedString::from(format!(
                        "graph-section-{}",
                        section.label()
                    )))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py(px(5.))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.bg_alt))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_section(section, cx)))
                    .child(
                        div()
                            .w(px(12.))
                            .text_color(theme.text_muted)
                            .child(if collapsed { "▸" } else { "▾" }),
                    )
                    .child(div().text_color(theme.text).child(section.label())),
            )
            .when(!collapsed, |d| {
                d.child(div().flex().flex_col().pb_1().children(body))
            })
    }

    fn render_controls(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        if self.config.close.unwrap_or(false) {
            return Self::icon_button("graph-controls-open", "⚙", theme)
                .absolute()
                .top_2()
                .left_2()
                .bg(theme.bg_secondary)
                .border_1()
                .border_color(theme.border)
                .occlude()
                .on_click(cx.listener(|this, _, _, cx| this.set_closed(false, cx)))
                .into_any_element();
        }
        let c = &self.config;
        let mut filters: Vec<gpui::AnyElement> = vec![
            div()
                .px_2()
                .py(px(3.))
                .child(self.search.clone())
                .into_any_element(),
            self.flag_row("Tags", "showTags", c.show_tags.unwrap_or(false), theme, cx)
                .into_any_element(),
            self.flag_row(
                "Attachments",
                "showAttachments",
                c.show_attachments.unwrap_or(false),
                theme,
                cx,
            )
            .into_any_element(),
            self.flag_row(
                "Existing files only",
                "hideUnresolved",
                c.hide_unresolved.unwrap_or(false),
                theme,
                cx,
            )
            .into_any_element(),
            self.flag_row(
                "Orphans",
                "showOrphans",
                c.show_orphans.unwrap_or(true),
                theme,
                cx,
            )
            .into_any_element(),
        ];
        if self.is_local() {
            filters.push(
                self.flag_row(
                    "Backlinks",
                    "localBacklinks",
                    self.local_backlinks,
                    theme,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.local_backlinks = !this.local_backlinks;
                    this.rebuild(cx);
                }))
                .into_any_element(),
            );
            filters.push(
                self.flag_row(
                    "Forelinks",
                    "localForelinks",
                    self.local_forelinks,
                    theme,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.local_forelinks = !this.local_forelinks;
                    this.rebuild(cx);
                }))
                .into_any_element(),
            );
            filters.push(
                self.flag_row(
                    "Interlinks",
                    "localInterlinks",
                    self.local_interlinks,
                    theme,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.local_interlinks = !this.local_interlinks;
                    this.rebuild(cx);
                }))
                .into_any_element(),
            );
        }
        let mut groups: Vec<gpui::AnyElement> = c
            .color_groups
            .iter()
            .enumerate()
            .map(|(ix, group)| {
                let mut color: Hsla = gpui::rgb(group.color.rgb).into();
                color.a = group.color.a.clamp(0.0, 1.0);
                let editing = self.editing_group == Some(ix);
                let query: gpui::AnyElement = if editing {
                    self.group_field.clone().into_any_element()
                } else {
                    div()
                        .id(SharedString::from(format!("graph-group-query-{ix}")))
                        .flex_1()
                        .min_w_0()
                        .px_1()
                        .rounded_sm()
                        .cursor_pointer()
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_color(if group.query.is_empty() {
                            theme.text_faint
                        } else {
                            theme.text
                        })
                        .hover(|s| s.bg(theme.bg_alt))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.start_edit_group(ix, window, cx)
                        }))
                        .child(SharedString::from(if group.query.is_empty() {
                            "Enter query...".to_string()
                        } else {
                            group.query.clone()
                        }))
                        .into_any_element()
                };
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py(px(2.))
                    .child(
                        div()
                            .id(SharedString::from(format!("graph-group-color-{ix}")))
                            .flex_shrink_0()
                            .w(px(16.))
                            .h(px(16.))
                            .rounded_sm()
                            .cursor_pointer()
                            .bg(color)
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.cycle_group_color(ix, cx)),
                            ),
                    )
                    .child(query)
                    .child(
                        Self::icon_button_dyn(format!("graph-group-remove-{ix}"), "×", theme)
                            .on_click(cx.listener(move |this, _, _, cx| this.remove_group(ix, cx))),
                    )
                    .into_any_element()
            })
            .collect();
        groups.push(
            div()
                .px_2()
                .py(px(3.))
                .child(
                    div()
                        .id("graph-group-add")
                        .px_2()
                        .py(px(2.))
                        .rounded_sm()
                        .cursor_pointer()
                        .bg(theme.bg_alt)
                        .border_1()
                        .border_color(theme.border)
                        .text_color(theme.text)
                        .hover(|s| s.bg(theme.bg_active))
                        .on_click(cx.listener(|this, _, window, cx| this.add_group(window, cx)))
                        .child("New group"),
                )
                .into_any_element(),
        );
        let display: Vec<gpui::AnyElement> = vec![
            self.flag_row(
                "Arrows",
                "showArrow",
                c.show_arrow.unwrap_or(false),
                theme,
                cx,
            )
            .into_any_element(),
            self.slider_row(Slider::TextFade, theme, cx)
                .into_any_element(),
            self.slider_row(Slider::NodeSize, theme, cx)
                .into_any_element(),
            self.slider_row(Slider::LineSize, theme, cx)
                .into_any_element(),
            Self::icon_button_dyn("graph-refresh".to_string(), "↻", theme)
                .on_click(cx.listener(|this, _, _, cx| this.rebuild(cx)))
                .into_any_element(),
        ];
        let mut display = display;
        if self.is_local() {
            let depth_label = format!("Depth {}", self.local_depth);
            let depth_row = div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .px_2()
                .py(px(3.))
                .child(div().text_color(theme.text).child("Depth"))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .child(
                            Self::icon_button_dyn("graph-depth-down".to_string(), "−", theme)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if this.local_depth > 1 {
                                        this.local_depth -= 1;
                                        this.rebuild(cx);
                                    }
                                })),
                        )
                        .child(div().text_color(theme.text).child(depth_label))
                        .child(
                            Self::icon_button_dyn("graph-depth-up".to_string(), "+", theme)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.local_depth += 1;
                                    this.rebuild(cx);
                                })),
                        ),
                )
                .into_any_element();
            display.push(depth_row);
        }
        let forces: Vec<gpui::AnyElement> = vec![
            self.slider_row(Slider::Center, theme, cx)
                .into_any_element(),
            self.slider_row(Slider::Repel, theme, cx).into_any_element(),
            self.slider_row(Slider::Link, theme, cx).into_any_element(),
            self.slider_row(Slider::LinkDistance, theme, cx)
                .into_any_element(),
        ];
        div()
            .id("graph-controls")
            .occlude()
            .absolute()
            .top_2()
            .left_2()
            .w(px(260.))
            .max_h_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .bg(theme.bg_secondary)
            .border_1()
            .border_color(theme.border)
            .rounded_md()
            .text_size(px(12.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_end()
                    .gap_1()
                    .px_1()
                    .py(px(3.))
                    .child(
                        Self::icon_button("graph-controls-animate", "▶", theme)
                            .on_click(cx.listener(|this, _, _, cx| this.restart_layout(cx))),
                    )
                    .child(
                        Self::icon_button("graph-controls-reset", "↺", theme).on_click(
                            cx.listener(|this, _, _, cx| {
                                // Randomize the physics layout without touching zoom or pan.
                                // user_adjusted = true keeps the simulation from calling fit()
                                // and overriding the user's current view.
                                let mut layout = Layout::new(
                                    this.nodes.len(),
                                    (*this.edges).clone(),
                                    this.forces(),
                                );
                                for (i, node) in this.nodes.iter().enumerate() {
                                    layout.bodies[i].mass = 1.0 + (node.degree as f32).sqrt() * 0.5;
                                }
                                this.layout = Arc::new(Mutex::new(layout));
                                this.running = false;
                                this.user_adjusted = true;
                                this.start_simulation(cx);
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        Self::icon_button("graph-controls-close", "×", theme)
                            .on_click(cx.listener(|this, _, _, cx| this.set_closed(true, cx))),
                    ),
            )
            .child(self.section(Section::Filters, filters, theme, cx))
            .child(self.section(Section::Groups, groups, theme, cx))
            .child(self.section(Section::Display, display, theme, cx))
            .child(self.section(Section::Forces, forces, theme, cx))
            .into_any_element()
    }

    fn icon_button_dyn(
        name: String,
        glyph: &'static str,
        theme: &Theme,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(SharedString::from(name))
            .w(px(18.))
            .h(px(18.))
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .rounded_sm()
            .cursor_pointer()
            .text_color(theme.text_muted)
            .hover(|s| s.bg(theme.bg_alt).text_color(theme.text))
            .child(glyph)
    }
}

/// The scale and pan that frame the world rectangle `(x0, y0, x1, y1)` in an
/// element `w` by `h` pixels, or `None` while the element has no size yet. A
/// `pinned` zoom is kept as it is and only the centering is computed.
/// The zoom a `graph.json` `scale` pins the camera to. Obsidian writes
/// `scale: 1` into every graph.json it creates, so only a value away from
/// that default is a zoom the user chose; honoring 1 would pin a big vault at
/// a scale that leaves most of it off screen.
fn saved_pin(saved: Option<f32>) -> Option<f32> {
    saved
        .map(|scale| scale.clamp(0.1, 8.0))
        .filter(|scale| (scale - 1.0).abs() > 0.001)
}

/// The zoom factor a wheel event multiplies the scale by. A wheel-up event,
/// `dy` positive in the same convention that scrolls the editor up, zooms the
/// graph in, like `Ctrl+=` and like Obsidian.
fn wheel_zoom_factor(dy: f32) -> f32 {
    (dy * 0.004).exp()
}

/// Whether a refresh takes its zoom from `graph.json`. The vault refresh runs
/// on every change, so the saved zoom is adopted once — the first refresh
/// that sees that value — and a camera the user has taken over, or reset with
/// `Ctrl+0`, is left alone afterwards. A value that has since changed on disk
/// is a fresh intention and is adopted again.
fn adopts_saved_scale(saved: Option<f32>, adopted: Option<f32>, user_adjusted: bool) -> bool {
    !user_adjusted && saved != adopted
}

fn frame(
    w: f32,
    h: f32,
    (x0, y0, x1, y1): (f32, f32, f32, f32),
    pinned: Option<f32>,
) -> Option<(f32, Point<f32>)> {
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let bw = (x1 - x0).max(1.0) + 120.0;
    let bh = (y1 - y0).max(1.0) + 120.0;
    let scale = pinned.unwrap_or_else(|| (w / bw).min(h / bh).clamp(0.05, 1.5));
    let cx = (x0 + x1) / 2.0;
    let cy = (y0 + y1) / 2.0;
    Some((scale, point(-cx * scale, -cy * scale)))
}

/// Builds the node and edge lists for the current filters.
fn build(
    state: &VaultState,
    config: &GraphConfig,
    local: Option<&str>,
    local_depth: usize,
    local_forelinks: bool,
    local_backlinks: bool,
    local_interlinks: bool,
    color_groups: &[(Arc<Search>, Hsla)],
) -> Built {
    let cache = state.cache();
    let vault = state.vault();
    let texts = state.texts();
    let show_attachments = config.show_attachments.unwrap_or(false);
    let show_tags = config.show_tags.unwrap_or(false);
    let show_orphans = config.show_orphans.unwrap_or(true);
    let hide_unresolved = config.hide_unresolved.unwrap_or(false);

    let mut nodes: Vec<Node> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    fn add(
        index: &mut HashMap<String, usize>,
        nodes: &mut Vec<Node>,
        id: String,
        label: String,
        kind: NodeKind,
    ) -> usize {
        if let Some(&ix) = index.get(&id) {
            return ix;
        }
        let ix = nodes.len();
        index.insert(id.clone(), ix);
        nodes.push(Node {
            id,
            label: label.into(),
            kind,
            degree: 0,
            color: None,
        });
        ix
    }
    let label_of = |path: &str| -> String {
        let name = path.rsplit('/').next().unwrap_or(path);
        if FileKind::from_path(path) == FileKind::Markdown {
            name.trim_end_matches(".md").to_string()
        } else {
            name.to_string()
        }
    };

    // The search filter from graph.json narrows the note set.
    let filter = config
        .search
        .as_deref()
        .filter(|q| !q.trim().is_empty())
        .and_then(|q| Search::new(q).ok());
    let passes = |path: &str| -> bool {
        let Some(search) = &filter else { return true };
        let (Some(meta), Some(text)) = (cache.file(path), texts.get(path)) else {
            return true;
        };
        let (tags, blocks, sections, tasks) = inputs(meta, text);
        search
            .matches(&SearchFile {
                path,
                content: text,
                tags: &tags,
                frontmatter: meta.frontmatter.as_ref(),
                blocks: &blocks,
                sections: &sections,
                tasks: &tasks,
            })
            .is_some()
    };

    for (path, _) in cache.files() {
        if passes(path) {
            add(
                &mut index,
                &mut nodes,
                path.to_string(),
                label_of(path),
                NodeKind::Note,
            );
        }
    }
    if show_attachments {
        for file in vault.files() {
            if file.kind != FileKind::Markdown && file.kind != FileKind::Other {
                add(
                    &mut index,
                    &mut nodes,
                    file.path.clone(),
                    label_of(&file.path),
                    NodeKind::Attachment,
                );
            }
        }
    }
    // Obsidian's `localFile` is independent of `showAttachments` and of
    // `showTags`: a local graph is centred on the file the user has open,
    // whether or not the filters would show that file in the global graph.
    // Without this the centre is missing from the index and the whole
    // graph comes back empty.
    if let Some(center) = local {
        let kind = if FileKind::from_path(center) == FileKind::Markdown {
            NodeKind::Note
        } else {
            NodeKind::Attachment
        };
        add(
            &mut index,
            &mut nodes,
            center.to_string(),
            label_of(center),
            kind,
        );
    }

    let mut edge_set: HashSet<(u32, u32)> = HashSet::new();
    let note_paths: Vec<String> = cache.files().map(|(p, _)| p.to_string()).collect();
    for path in &note_paths {
        let Some(&from) = index.get(path) else {
            continue;
        };
        if let Some(targets) = cache.resolved_links(path) {
            for target in targets.keys() {
                let kind = FileKind::from_path(target);
                // An edge into the centre is drawn whatever the filters
                // say about its kind, for the same reason the node is.
                let is_center = local == Some(target.as_str());
                if kind != FileKind::Markdown && !show_attachments && !is_center {
                    continue;
                }
                let to = match index.get(target) {
                    Some(&ix) => ix,
                    None if kind == FileKind::Markdown => continue, // filtered out
                    None => add(
                        &mut index,
                        &mut nodes,
                        target.clone(),
                        label_of(target),
                        NodeKind::Attachment,
                    ),
                };
                if from != to {
                    edge_set.insert((from as u32, to as u32));
                }
            }
        }
        if !hide_unresolved {
            if let Some(targets) = cache.unresolved_links(path) {
                for target in targets.keys() {
                    if target.is_empty() {
                        continue;
                    }
                    let to = add(
                        &mut index,
                        &mut nodes,
                        format!("unresolved:{}", target.to_lowercase()),
                        target.clone(),
                        NodeKind::Unresolved,
                    );
                    edge_set.insert((from as u32, to as u32));
                }
            }
        }
        if show_tags {
            if let Some(meta) = cache.file(path) {
                for tag in silica_cache::all_tags(meta) {
                    let to = add(
                        &mut index,
                        &mut nodes,
                        format!("tag:{}", tag.to_lowercase()),
                        format!("#{tag}"),
                        NodeKind::Tag,
                    );
                    edge_set.insert((from as u32, to as u32));
                }
            }
        }
    }

    // Color groups: pre-compiled in GraphView::rebuild(), passed in as
    // color_groups. First matching query wins. A group with an empty query
    // is dropped before being passed in.
    // Until 2026-10-28 this was not stated anywhere and the group coloured
    // every note in the graph, because an `All` node with no children used to
    // be vacuously true in the matcher. That default is gone.
    if !color_groups.is_empty() {
        for node in nodes.iter_mut().filter(|n| n.kind == NodeKind::Note) {
            let (Some(meta), Some(text)) = (cache.file(&node.id), texts.get(&node.id)) else {
                continue;
            };
            let (tags, blocks, sections, tasks) = inputs(meta, text);
            let file = SearchFile {
                path: &node.id,
                content: text,
                tags: &tags,
                frontmatter: meta.frontmatter.as_ref(),
                blocks: &blocks,
                sections: &sections,
                tasks: &tasks,
            };
            node.color = color_groups
                .iter()
                .find(|(s, _)| s.matches(&file).is_some())
                .map(|(_, c)| *c);
        }
    }

    let mut edges: Vec<(u32, u32)> = edge_set.clone().into_iter().collect();
    edges.sort();

    // Local graph: the centre and its neighbours out to `local_depth`.
    if let Some(center) = local {
        // Added above whether the filters kept it or not, so the only way
        // this misses is a centre that is not a vault path at all.
        let Some(&c) = index.get(center) else {
            return Built {
                nodes: Vec::new(),
                edges: Vec::new(),
                neighbors: Vec::new(),
            };
        };
        let mut keep: HashSet<usize> = HashSet::new();
        let mut depth_of: HashMap<usize, usize> = HashMap::new();
        let mut queue: VecDeque<(usize, usize)> = VecDeque::new();
        queue.push_back((c, 0));
        keep.insert(c);
        depth_of.insert(c, 0);
        while let Some((n, depth)) = queue.pop_front() {
            if depth >= local_depth {
                continue;
            }
            let next_depth = depth + 1;
            for &(a, b) in &edges {
                let (a, b) = (a as usize, b as usize);
                // Direction: forelinks = n is the source, backlinks = n is the target.
                let is_out = local_forelinks && a == n;
                let is_in = local_backlinks && b == n;
                if !is_out && !is_in {
                    continue;
                }
                let other = if is_out { b } else { a };
                if keep.insert(other) {
                    depth_of.insert(other, next_depth);
                    queue.push_back((other, next_depth));
                }
            }
        }
        // Interlinks: when off, drop edges between nodes at the same depth
        // (edges that skip levels, not passing through the centre).
        let edges: Vec<(u32, u32)> = if local_interlinks {
            edge_set.into_iter().collect()
        } else {
            edge_set
                .into_iter()
                .filter(|&(a, b)| {
                    let (a, b) = (a as usize, b as usize);
                    let Some(&da) = depth_of.get(&a) else {
                        return false;
                    };
                    let Some(&db) = depth_of.get(&b) else {
                        return false;
                    };
                    da != db
                })
                .collect()
        };
        return finish(nodes, edges, Some(keep), true);
    }
    finish(nodes, edges, None, show_orphans)
}

/// Drops nodes outside `keep` and, unless `show_orphans`, nodes without
/// edges; renumbers edges and counts degrees.
fn finish(
    nodes: Vec<Node>,
    edges: Vec<(u32, u32)>,
    keep: Option<HashSet<usize>>,
    show_orphans: bool,
) -> Built {
    let mut degree = vec![0usize; nodes.len()];
    for &(a, b) in &edges {
        degree[a as usize] += 1;
        degree[b as usize] += 1;
    }
    let mut remap = vec![usize::MAX; nodes.len()];
    let mut out_nodes = Vec::new();
    for (i, mut node) in nodes.into_iter().enumerate() {
        let kept = keep.as_ref().is_none_or(|k| k.contains(&i));
        let orphan = degree[i] == 0;
        if !kept || (orphan && !show_orphans) {
            continue;
        }
        node.degree = degree[i];
        remap[i] = out_nodes.len();
        out_nodes.push(node);
    }
    let mut out_edges = Vec::new();
    let mut neighbors = vec![Vec::new(); out_nodes.len()];
    for (a, b) in edges {
        let (a, b) = (remap[a as usize], remap[b as usize]);
        if a == usize::MAX || b == usize::MAX {
            continue;
        }
        out_edges.push((a as u32, b as u32));
        neighbors[a].push(b);
        neighbors[b].push(a);
    }
    for (i, node) in out_nodes.iter_mut().enumerate() {
        node.degree = neighbors[i].len();
    }
    Built {
        nodes: out_nodes,
        edges: out_edges,
        neighbors,
    }
}

impl Render for GraphView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A group field that stopped being drawn while it held the keyboard
        // would leave the keys outside every context; the graph takes them.
        if self.editing_group.is_none() && self.group_field.focus_handle(cx).is_focused(window) {
            window.focus(&self.focus_handle, cx);
        }
        let theme = cx.global::<Theme>().clone();
        let entity = cx.entity();
        let paint_entity = entity.clone();
        let node_count = self.nodes.len();
        let edge_count = self.edges.len();
        let local = self.is_local();
        let hovered_label = self
            .hovered
            .and_then(|ix| self.nodes.get(ix))
            .map(|n| n.label.clone());
        let status: SharedString = if self.building {
            "Building graph…".into()
        } else {
            format!("{node_count} nodes, {edge_count} links").into()
        };

        let chip = |label: SharedString, on: bool, theme: &Theme| {
            div()
                .px_2()
                .py_px()
                .rounded_sm()
                .text_size(px(11.))
                .cursor_pointer()
                .bg(if on {
                    theme.bg_active
                } else {
                    theme.bg_secondary
                })
                .text_color(if on { theme.text } else { theme.text_muted })
                .border_1()
                .border_color(theme.border)
                .child(label)
        };

        div()
            .key_context("GraphView")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::zoom_in))
            .on_action(cx.listener(Self::zoom_out))
            .on_action(cx.listener(Self::reset_view))
            .relative()
            .size_full()
            .bg(theme.bg)
            .child(
                canvas(
                    move |bounds, window, cx| {
                        entity.update(cx, |view, _| {
                            let sized =
                                view.bounds.size.width > px(0.) && view.bounds.size.height > px(0.);
                            view.bounds = bounds;
                            // fit() does nothing until the element has a size,
                            // so a layout that settles before the first paint
                            // would never be framed at all.
                            if !sized && !view.user_adjusted {
                                view.fit();
                            }
                        });
                        // The controls panel occludes: a press on one of its
                        // sliders must not also pan the graph underneath.
                        window.insert_hitbox(bounds, HitboxBehavior::Normal)
                    },
                    move |bounds, hitbox, window, cx| {
                        let theme = cx.global::<Theme>().clone();
                        let line_height = window.line_height();
                        let (
                            nodes,
                            edges,
                            positions,
                            radii,
                            hovered,
                            current_ix,
                            highlight,
                            scale,
                            line_mult,
                            label_from,
                            arrows,
                        ) = {
                            let view = paint_entity.read(cx);
                            let layout = view.layout.lock().unwrap_or_else(|e| e.into_inner());
                            let positions: Vec<Point<Pixels>> = layout
                                .bodies
                                .iter()
                                .map(|b| view.to_screen(b.x, b.y))
                                .collect();
                            let radii: Vec<f32> =
                                view.nodes.iter().map(|n| view.node_radius(n)).collect();
                            let hovered = view.hovered;
                            let current_ix = view
                                .current
                                .as_ref()
                                .and_then(|c| view.index.get(c))
                                .copied();
                            let highlight: HashSet<usize> = match hovered {
                                Some(h) => {
                                    let mut s: HashSet<usize> = view
                                        .neighbors
                                        .get(h)
                                        .cloned()
                                        .unwrap_or_default()
                                        .into_iter()
                                        .collect();
                                    s.insert(h);
                                    s
                                }
                                None => HashSet::new(),
                            };
                            (
                                view.nodes.clone(),
                                view.edges.clone(),
                                positions,
                                radii,
                                hovered,
                                current_ix,
                                highlight,
                                view.scale,
                                view.config.line_size_multiplier.unwrap_or(1.0),
                                view.label_threshold(),
                                view.config.show_arrow.unwrap_or(false),
                            )
                        };
                        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
                            // Edges: one stroked path for plain ones, one for highlighted.
                            let mut plain = PathBuilder::stroke(px(1.0 * line_mult));
                            let mut bright = PathBuilder::stroke(px(1.5 * line_mult));
                            // Arrowheads, filled, at the target end of each
                            // edge just outside the node.
                            let mut plain_heads = PathBuilder::fill();
                            let mut bright_heads = PathBuilder::fill();
                            let mut any_plain = false;
                            let mut any_bright = false;
                            let node_scale = scale.clamp(0.35, 1.6);
                            for &(a, b) in edges.iter() {
                                let (pa, pb) = (positions[a as usize], positions[b as usize]);
                                if !bounds.contains(&pa) && !bounds.contains(&pb) {
                                    continue;
                                }
                                let on = hovered.is_some()
                                    && highlight.contains(&(a as usize))
                                    && highlight.contains(&(b as usize));
                                let (line, heads) = if on {
                                    any_bright = true;
                                    (&mut bright, &mut bright_heads)
                                } else {
                                    any_plain = true;
                                    (&mut plain, &mut plain_heads)
                                };
                                line.move_to(pa);
                                line.line_to(pb);
                                if arrows {
                                    let r = radii[b as usize] * node_scale;
                                    if let Some([tip, left, right]) =
                                        arrowhead(pa, pb, r, 4.0 + 2.0 * line_mult)
                                    {
                                        heads.move_to(tip);
                                        heads.line_to(left);
                                        heads.line_to(right);
                                        heads.close();
                                    }
                                }
                            }
                            let mut edge_color = theme.text_faint;
                            edge_color.a = if hovered.is_some() { 0.25 } else { 0.6 };
                            if any_plain {
                                if let Ok(path) = plain.build() {
                                    window.paint_path(path, edge_color);
                                }
                                if arrows {
                                    if let Ok(path) = plain_heads.build() {
                                        window.paint_path(path, edge_color);
                                    }
                                }
                            }
                            if any_bright {
                                if let Ok(path) = bright.build() {
                                    window.paint_path(path, theme.accent);
                                }
                                if arrows {
                                    if let Ok(path) = bright_heads.build() {
                                        window.paint_path(path, theme.accent);
                                    }
                                }
                            }
                            // Nodes.
                            let style = window.text_style();
                            let font_size = px(11.);
                            let mut labels = 0;
                            for (i, node) in nodes.iter().enumerate() {
                                let p = positions[i];
                                let r = px(radii[i] * scale.clamp(0.35, 1.6));
                                if p.x + r < bounds.left()
                                    || p.x - r > bounds.right()
                                    || p.y + r < bounds.top()
                                    || p.y - r > bounds.bottom()
                                {
                                    continue;
                                }
                                let dim = hovered.is_some() && !highlight.contains(&i);
                                let mut color = match node.kind {
                                    NodeKind::Note => node.color.unwrap_or(theme.text_muted),
                                    NodeKind::Attachment => theme.text_faint,
                                    NodeKind::Unresolved => theme.text_faint,
                                    NodeKind::Tag => theme.tag,
                                };
                                if Some(i) == current_ix || Some(i) == hovered {
                                    color = theme.accent;
                                }
                                if dim {
                                    color.a *= 0.3;
                                }
                                let d = r * 2.;
                                let mut quad =
                                    fill(Bounds::new(point(p.x - r, p.y - r), size(d, d)), color);
                                quad.corner_radii = gpui::Corners::all(r);
                                if node.kind == NodeKind::Unresolved {
                                    quad.background = gpui::transparent_black().into();
                                    quad.border_widths = gpui::Edges::all(px(1.));
                                    quad.border_color = color;
                                }
                                window.paint_quad(quad);
                                // Labels fade in with zoom; the hovered node always shows.
                                let show_label = Some(i) == hovered
                                    || Some(i) == current_ix
                                    || (scale >= label_from && labels < 400 && !dim);
                                if show_label {
                                    labels += 1;
                                    let mut run = style.to_run(node.label.len());
                                    run.color = if dim { theme.text_faint } else { theme.text };
                                    let line = window.text_system().shape_line(
                                        node.label.clone(),
                                        font_size,
                                        &[run],
                                        None,
                                    );
                                    let origin = point(p.x - line.width / 2., p.y + r + px(3.));
                                    let _ = line.paint(
                                        origin,
                                        line_height,
                                        TextAlign::Left,
                                        None,
                                        window,
                                        cx,
                                    );
                                }
                            }
                        });

                        let e = paint_entity.clone();
                        let hb = hitbox.clone();
                        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                            if phase == DispatchPhase::Bubble
                                && event.button == MouseButton::Left
                                && hb.is_hovered(window)
                            {
                                e.update(cx, |v, cx| v.mouse_down(event, window, cx));
                            }
                        });
                        let e = paint_entity.clone();
                        let hb = hitbox.clone();
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                            if phase == DispatchPhase::Bubble {
                                // Over the panel nothing underneath is hovered.
                                let over = hb.is_hovered(window);
                                e.update(cx, |v, cx| {
                                    if over || v.drag.is_some() {
                                        v.mouse_move(event, cx);
                                    } else if v.hovered.take().is_some() {
                                        cx.notify();
                                    }
                                });
                            }
                        });
                        let e = paint_entity.clone();
                        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                            if phase == DispatchPhase::Bubble {
                                e.update(cx, |v, cx| v.mouse_up(event, cx));
                            }
                        });
                        let e = paint_entity.clone();
                        window.on_mouse_event(
                            move |event: &ScrollWheelEvent, phase, window, cx| {
                                if phase == DispatchPhase::Bubble
                                    && hitbox.should_handle_scroll(window)
                                {
                                    e.update(cx, |v, cx| v.scroll(event, line_height, cx));
                                }
                            },
                        );
                    },
                )
                .size_full(),
            )
            .child(self.render_controls(&theme, cx))
            .child(
                div()
                    .absolute()
                    .top_2()
                    .right_2()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .children(
                        hovered_label
                            .map(|l| div().text_size(px(11.)).text_color(theme.text).child(l)),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme.text_muted)
                            .child(status),
                    )
                    .child(
                        div()
                            .id("graph-local")
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_local(cx)))
                            .child(chip(
                                if local {
                                    "Local".into()
                                } else {
                                    "Global".into()
                                },
                                local,
                                &theme,
                            )),
                    ),
            )
    }
}

/// The three corners of an arrowhead on the edge from `from` to `to`, its
/// tip `radius` short of `to` so it sits on the node's rim; `None` for an
/// edge too short to carry one.
fn arrowhead(
    from: Point<Pixels>,
    to: Point<Pixels>,
    radius: f32,
    length: f32,
) -> Option<[Point<Pixels>; 3]> {
    let dx = f32::from(to.x) - f32::from(from.x);
    let dy = f32::from(to.y) - f32::from(from.y);
    let len = (dx * dx + dy * dy).sqrt();
    if len <= radius + length {
        return None;
    }
    let (ux, uy) = (dx / len, dy / len);
    let tip = (f32::from(to.x) - ux * radius, f32::from(to.y) - uy * radius);
    let base = (tip.0 - ux * length, tip.1 - uy * length);
    let half = length * 0.5;
    let left = (base.0 - uy * half, base.1 + ux * half);
    let right = (base.0 + uy * half, base.1 - ux * half);
    Some([
        point(px(tip.0), px(tip.1)),
        point(px(left.0), px(left.1)),
        point(px(right.0), px(right.1)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_graph_json_has_obsidians_twenty_keys_in_its_order() {
        let doc = full_document(&GraphConfig::default());
        let keys: Vec<&str> = doc
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "collapse-filter",
                "search",
                "showTags",
                "showAttachments",
                "hideUnresolved",
                "showOrphans",
                "collapse-color-groups",
                "colorGroups",
                "collapse-display",
                "showArrow",
                "textFadeMultiplier",
                "nodeSizeMultiplier",
                "lineSizeMultiplier",
                "collapse-forces",
                "centerStrength",
                "repelStrength",
                "linkStrength",
                "linkDistance",
                "scale",
                "close",
            ]
        );
        let text = serde_json::to_string_pretty(&doc).unwrap();
        assert!(
            text.contains("\"centerStrength\": 0.518713248970312"),
            "{text}"
        );
        assert!(text.contains("\"repelStrength\": 10"), "{text}");
        assert!(text.contains("\"textFadeMultiplier\": 0,"), "{text}");
        assert_eq!(text.len(), 494, "the bytes Obsidian wrote");
    }

    #[test]
    fn the_last_group_removed_is_written_as_an_empty_list() {
        let config = GraphConfig::default();
        let doc = merge_document(&config);
        assert_eq!(doc["colorGroups"], json!([]));
        assert_eq!(doc["search"], json!(""));
    }

    #[test]
    fn a_whole_number_is_written_without_a_decimal_point() {
        let config = GraphConfig {
            link_distance: Some(130.0),
            center_strength: Some(0.25),
            ..GraphConfig::default()
        };
        let text = serde_json::to_string(&merge_document(&config)).unwrap();
        assert!(text.contains("\"linkDistance\":130,"), "{text}");
        assert!(text.contains("\"centerStrength\":0.25"), "{text}");
        let text = serde_json::to_string(&full_document(&config)).unwrap();
        assert!(text.contains("\"linkDistance\":130,"), "{text}");
        let config = GraphConfig {
            color_groups: vec![GraphColorGroup {
                query: "tag:#x".into(),
                color: GraphColor {
                    a: 1.0,
                    rgb: 0xe93147,
                },
                extra: Default::default(),
            }],
            ..GraphConfig::default()
        };
        for doc in [merge_document(&config), full_document(&config)] {
            let text = serde_json::to_string(&doc).unwrap();
            assert!(text.contains("\"a\":1,\"rgb\":15282503"), "{text}");
        }
    }

    #[test]
    fn a_slider_snaps_to_its_step_and_stays_in_range() {
        assert_eq!(Slider::Center.value_at(0.0), 0.0);
        assert_eq!(Slider::Center.value_at(1.0), 1.0);
        assert_eq!(Slider::Center.value_at(0.333), 0.33);
        assert_eq!(Slider::LinkDistance.value_at(0.5), 265.0);
        assert_eq!(Slider::TextFade.value_at(0.5), 0.0);
        assert_eq!(Slider::NodeSize.value_at(2.0), 5.0);
        assert_eq!(Slider::Repel.value_at(-1.0), 0.0);
        for slider in [Slider::Center, Slider::Repel, Slider::LinkDistance] {
            let v = slider.value_at(0.25);
            assert!((slider.fraction(v) - 0.25).abs() < 0.02, "{slider:?}: {v}");
        }
    }

    fn restored_defaults(config: &GraphConfig) -> GraphConfig {
        GraphConfig {
            show_tags: Some(false),
            link_distance: config.link_distance.or(Some(250.0)),
            ..config.clone()
        }
    }

    #[test]
    fn restore_keeps_the_panel_state_and_the_camera() {
        let mut config = GraphConfig {
            show_tags: Some(true),
            scale: Some(3.0),
            close: Some(true),
            ..GraphConfig::default()
        };
        config.extra.insert("collapse-forces".into(), json!(false));
        let restored = restored_defaults(&config);
        assert_eq!(restored.show_tags, Some(false));
        assert_eq!(restored.scale, Some(3.0));
        assert_eq!(restored.close, Some(true));
        assert_eq!(restored.extra["collapse-forces"], json!(false));
        assert_eq!(restored.link_distance, Some(250.0));
    }

    #[test]
    fn an_arrowhead_sits_on_the_target_rim_pointing_along_the_edge() {
        let [tip, left, right] =
            arrowhead(point(px(0.), px(0.)), point(px(100.), px(0.)), 10.0, 6.0).unwrap();
        assert_eq!(tip, point(px(90.), px(0.)));
        assert_eq!(left.x, px(84.));
        assert_eq!(right.x, px(84.));
        assert_eq!(left.y, px(3.));
        assert_eq!(right.y, px(-3.));
        assert!(arrowhead(point(px(0.), px(0.)), point(px(12.), px(0.)), 10.0, 6.0).is_none());
    }

    fn node(id: &str, kind: NodeKind) -> Node {
        Node {
            id: id.to_string(),
            label: id.to_string().into(),
            kind,
            degree: 0,
            color: None,
        }
    }

    /// Obsidian's Orphans filter is about a node's degree, not about whether
    /// it is a note: an attachment nothing links to goes with the rest.
    #[test]
    fn orphans_off_drops_unlinked_attachments_too() {
        let nodes = vec![
            node("Linker.md", NodeKind::Note),
            node("used.png", NodeKind::Attachment),
            node("lonely.png", NodeKind::Attachment),
            node("Alone.md", NodeKind::Note),
        ];
        let built = finish(nodes, vec![(0, 1)], None, false);
        let kept: Vec<&str> = built.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(kept, ["Linker.md", "used.png"]);
        assert_eq!(built.edges, [(0, 1)]);
    }

    /// Obsidian's `localFile` is independent of the filters: a local graph
    /// centred on an attachment draws it with "Attachments" off, where
    /// before the centre was missing from the index and the graph came
    /// back empty.
    #[test]
    fn a_local_centre_the_filters_hide_is_still_drawn() {
        let nodes = vec![
            node("Note.md", NodeKind::Note),
            node("lonely.png", NodeKind::Attachment),
        ];
        // The centre alone, kept even though nothing links to it.
        let built = finish(nodes, Vec::new(), Some([1].into_iter().collect()), true);
        let kept: Vec<&str> = built.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(kept, ["lonely.png"]);
    }

    #[test]
    fn orphans_on_keeps_every_node() {
        let nodes = vec![
            node("Linker.md", NodeKind::Note),
            node("used.png", NodeKind::Attachment),
            node("lonely.png", NodeKind::Attachment),
            node("Alone.md", NodeKind::Note),
        ];
        let built = finish(nodes, vec![(0, 1)], None, true);
        assert_eq!(built.nodes.len(), 4);
        assert_eq!(built.nodes[2].degree, 0);
    }

    #[test]
    fn a_pinned_zoom_survives_the_fit() {
        let (scale, offset) = frame(800.0, 600.0, (-100.0, -50.0, 100.0, 50.0), Some(4.0)).unwrap();
        assert_eq!(scale, 4.0);
        assert_eq!(offset, point(0.0, 0.0));
        // Off-center bounds still get centered, at the pinned scale.
        let (scale, offset) = frame(800.0, 600.0, (0.0, 0.0, 200.0, 100.0), Some(4.0)).unwrap();
        assert_eq!(scale, 4.0);
        assert_eq!(offset, point(-400.0, -200.0));
    }

    #[test]
    fn without_a_pinned_zoom_the_layout_is_framed() {
        let (scale, offset) = frame(800.0, 600.0, (-500.0, -500.0, 500.0, 500.0), None).unwrap();
        assert!((scale - 600.0 / 1120.0).abs() < 1e-6, "{scale}");
        assert_eq!(offset, point(0.0, 0.0));
    }

    /// Obsidian's own graph.json always carries `scale`, `1` when it created
    /// the file, so only a value away from 1 is a zoom to honor.
    #[test]
    fn only_a_scale_away_from_the_default_pins_the_camera() {
        assert_eq!(saved_pin(None), None);
        assert_eq!(saved_pin(Some(1.0)), None);
        assert_eq!(saved_pin(Some(4.0)), Some(4.0));
        assert_eq!(saved_pin(Some(99.0)), Some(8.0));
        assert_eq!(saved_pin(Some(0.0)), Some(0.1));
    }

    /// A vault change refreshes the graph, and the refresh must not undo
    /// Ctrl+0 by pinning the camera back to the zoom in graph.json.
    #[test]
    fn a_reset_outlives_the_next_refresh() {
        // First sight of the saved zoom: adopt it.
        assert!(adopts_saved_scale(Some(4.0), None, false));
        // Every later refresh leaves the camera where it is, whether the
        // reset cleared the pin or the user zoomed by hand.
        assert!(!adopts_saved_scale(Some(4.0), Some(4.0), false));
        assert!(!adopts_saved_scale(Some(4.0), None, true));
        // A vault with no saved zoom never adopts one.
        assert!(!adopts_saved_scale(None, None, false));
        // The file changed under us: that is a new intention.
        assert!(adopts_saved_scale(Some(2.0), Some(4.0), false));
    }

    /// Obsidian zooms the graph in when the wheel goes up, the direction
    /// `Ctrl+=` takes too.
    #[test]
    fn the_wheel_zooms_in_when_it_goes_up() {
        assert!(wheel_zoom_factor(120.0) > 1.0);
        assert!(wheel_zoom_factor(-120.0) < 1.0);
        assert_eq!(wheel_zoom_factor(0.0), 1.0);
    }

    #[test]
    fn an_element_with_no_size_yet_is_not_framed() {
        assert!(frame(0.0, 600.0, (-10.0, -10.0, 10.0, 10.0), None).is_none());
        assert!(frame(800.0, 0.0, (-10.0, -10.0, 10.0, 10.0), Some(4.0)).is_none());
    }
}
