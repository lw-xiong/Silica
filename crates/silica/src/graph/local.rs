//! The local graph pane in the right dock: the open note at the centre and
//! the notes one hop out, linked either way.
//!
//! The node and edge lists come from the same builder the graph view uses,
//! asked for the centre with depth 1, so the filters and colour groups in
//! `graph.json` shape this pane exactly as they shape the full graph. What
//! is deliberately unlike the graph view: no controls panel, no pan or
//! zoom, and the forces are run to settled on the background executor
//! instead of animated — a pane beside the note has a handful of nodes and
//! nothing to look around in.

use super::layout::Layout;
use super::view::{self, GraphEvent, LabelCache, Node, NodeKind};
use crate::theme::Theme;
use crate::workspace::VaultState;
use gpui::{
    canvas, div, fill, point, prelude::*, px, App, Bounds, Context, EventEmitter, FocusHandle,
    Focusable, HitboxBehavior, MouseButton, MouseDownEvent, MouseMoveEvent, PathBuilder, Pixels,
    Point, ShapedLine, Window,
};
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

/// How far from the open note the pane reaches, in hops.
const DEPTH: usize = 1;

/// Steps the simulation is allowed before the pane draws what it has: a
/// depth-one graph cools in a fraction of this, but a note linked to half
/// the vault must not hold the dock open.
const MAX_STEPS: u32 = 240;

/// World units of room left around the fitted graph, so a node on the edge
/// and the label under it are not on the pane's border.
const PAD_X: f32 = 90.0;
const PAD_Y: f32 = 60.0;

/// The pane's own focus ring, so the workspace's pane keys have something
/// to be focused on even though the pane has no rows to walk.
pub struct LocalGraph {
    focus_handle: FocusHandle,
    state: Option<Arc<VaultState>>,
    /// The note at the centre, when one is open.
    current: Option<String>,
    /// The `graph.json` the last rebuild read: the forces, the node size
    /// and the colour groups are all the pane takes from it.
    config: silica_formats::config::GraphConfig,
    nodes: Arc<Vec<Node>>,
    edges: Arc<Vec<(u32, u32)>>,
    neighbors: Arc<Vec<Vec<usize>>>,
    index: HashMap<String, usize>,
    layout: Arc<Mutex<Layout>>,
    labels: LabelCache,
    hovered: Option<usize>,
    /// The projection the last paint used. The mouse events run after it,
    /// so a hit test can be made in screen pixels.
    projection: Option<Projection>,
    generation: u64,
    building: bool,
}

impl EventEmitter<GraphEvent> for LocalGraph {}

impl Focusable for LocalGraph {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// Where a world point lands on screen: `ox + (x - wx) * scale`.
#[derive(Debug, Clone, Copy)]
struct Projection {
    scale: f32,
    ox: f32,
    oy: f32,
    wx: f32,
    wy: f32,
}

impl Projection {
    fn to_screen(self, x: f32, y: f32) -> Point<Pixels> {
        point(
            px(self.ox + (x - self.wx) * self.scale),
            px(self.oy + (y - self.wy) * self.scale),
        )
    }

    fn to_world(self, p: Point<Pixels>) -> (f32, f32) {
        (
            (f32::from(p.x) - self.ox) / self.scale + self.wx,
            (f32::from(p.y) - self.oy) / self.scale + self.wy,
        )
    }
}

impl LocalGraph {
    pub fn new(cx: &mut Context<Self>) -> LocalGraph {
        LocalGraph {
            focus_handle: cx.focus_handle(),
            state: None,
            current: None,
            config: silica_formats::config::GraphConfig::default(),
            nodes: Arc::new(Vec::new()),
            edges: Arc::new(Vec::new()),
            neighbors: Arc::new(Vec::new()),
            index: HashMap::new(),
            layout: Arc::new(Mutex::new(Layout::new(
                0,
                Vec::new(),
                view::forces(&silica_formats::config::GraphConfig::default()),
            ))),
            labels: Arc::new(Mutex::new(HashMap::new())),
            hovered: None,
            projection: None,
            generation: 0,
            building: false,
        }
    }

    pub fn set_state(&mut self, state: Arc<VaultState>, cx: &mut Context<Self>) {
        self.state = Some(state);
        self.rebuild(cx);
    }

    /// The note the pane is centred on. Changing it is a new graph, so the
    /// rebuild runs again rather than the highlight moving.
    pub fn set_current(&mut self, path: Option<String>, cx: &mut Context<Self>) {
        if self.current == path {
            return;
        }
        self.current = path;
        self.rebuild(cx);
    }

    /// The centre, the one hop out, and nothing further: `build` does the
    /// walk, so the pane's depth is the builder's depth.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let Some(center) = self.current.clone() else {
            // * No note is open, so there is no centre to reach one hop from.
            self.clear(cx);
            return;
        };
        self.config = state.vault().graph_config().unwrap_or_default();
        let config = self.config.clone();
        let groups = view::color_groups(&config);
        let forces = view::forces(&config);
        // * Where a node that survives was, so switching notes in the pane
        //   does not re-roll the ones that stay.
        let old = self.positions();
        self.generation += 1;
        let generation = self.generation;
        self.building = true;
        cx.spawn(async move |this, cx| {
            let built = cx
                .background_executor()
                .spawn(async move {
                    let built = view::build(
                        &state,
                        &config,
                        Some(center.as_str()),
                        DEPTH,
                        true,
                        true,
                        false,
                        &groups,
                    );
                    let layout = simulate(&built, forces, &old);
                    built.into_layout(layout)
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

    fn clear(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        self.nodes = Arc::new(Vec::new());
        self.edges = Arc::new(Vec::new());
        self.neighbors = Arc::new(Vec::new());
        self.index = HashMap::new();
        self.layout = Arc::new(Mutex::new(Layout::new(
            0,
            Vec::new(),
            view::forces(&self.config),
        )));
        self.labels = Arc::new(Mutex::new(HashMap::new()));
        self.hovered = None;
        self.projection = None;
        self.building = false;
        cx.notify();
    }

    /// Node ids with where the running layout has them.
    fn positions(&self) -> HashMap<String, (f32, f32)> {
        let layout = self.layout.lock().unwrap_or_else(|e| e.into_inner());
        self.nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| layout.bodies.get(i).map(|b| (n.id.clone(), (b.x, b.y))))
            .collect()
    }

    fn apply(&mut self, built: view::Placed, cx: &mut Context<Self>) {
        self.index = built
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.clone(), i))
            .collect();
        self.nodes = Arc::new(built.nodes);
        self.edges = Arc::new(built.edges);
        self.neighbors = Arc::new(built.neighbors);
        // * The node set changed, so a shaped line for a node that is gone
        //   is a stale entry: drop them and let the next paint shape the
        //   new names once.
        self.labels = Arc::new(Mutex::new(HashMap::new()));
        self.layout = Arc::new(Mutex::new(built.layout));
        self.hovered = None;
        cx.notify();
    }

    /// The node under `p`, the nearest one within its radius plus a little.
    fn hit_test(&self, p: Point<Pixels>) -> Option<usize> {
        let projection = self.projection?;
        let layout = self.layout.lock().unwrap_or_else(|e| e.into_inner());
        let (wx, wy) = projection.to_world(p);
        let mut best: Option<(usize, f32)> = None;
        for (i, node) in self.nodes.iter().enumerate() {
            let Some(b) = layout.bodies.get(i) else {
                continue;
            };
            let r = (self.node_radius(node) + 4.0) / projection.scale.max(0.01);
            let d2 = (b.x - wx).powi(2) + (b.y - wy).powi(2);
            if d2 <= r * r && best.is_none_or(|(_, bd)| d2 < bd) {
                best = Some((i, d2));
            }
        }
        best.map(|(i, _)| i)
    }

    fn node_radius(&self, node: &Node) -> f32 {
        let mult = self.config.node_size_multiplier.unwrap_or(1.0);
        (2.5 + (node.degree as f32).sqrt() * 1.2) * mult
    }

    /// Scale and centre that fit the layout in `bounds`, or `None` before
    /// there is anything to fit.
    fn projection_for(&self, bounds: Bounds<Pixels>) -> Option<Projection> {
        let layout = self.layout.lock().unwrap_or_else(|e| e.into_inner());
        if layout.bodies.is_empty() {
            return None;
        }
        let (x0, y0, x1, y1) = layout.bounds();
        let w = f32::from(bounds.size.width).max(1.0);
        let h = f32::from(bounds.size.height).max(1.0);
        let span_x = (x1 - x0).max(1.0) + 2.0 * PAD_X;
        let span_y = (y1 - y0).max(1.0) + 2.0 * PAD_Y;
        let scale = (w / span_x).min(h / span_y).clamp(0.1, 4.0);
        Some(Projection {
            scale,
            ox: f32::from(bounds.center().x),
            oy: f32::from(bounds.center().y),
            wx: (x0 + x1) / 2.0,
            wy: (y0 + y1) / 2.0,
        })
    }

    fn hover(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let hit = self.hit_test(position);
        if hit != self.hovered {
            self.hovered = hit;
            cx.notify();
        }
    }

    /// A click on a node: the note it names opens, the way a click in the
    /// graph view does. Unresolved links and tags have no file to open, so
    /// they are left alone.
    fn click(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(ix) = self.hit_test(position) else {
            return;
        };
        if let Some(node) = self.nodes.get(ix) {
            if matches!(node.kind, NodeKind::Note | NodeKind::Attachment) {
                cx.emit(GraphEvent::Open(node.id.clone()));
            }
        }
    }
}

/// Runs the forces over `built` until they cool, seeding the nodes that
/// were where `old` left them.
fn simulate(
    built: &view::Built,
    forces: super::layout::Forces,
    old: &HashMap<String, (f32, f32)>,
) -> Layout {
    let mut layout = Layout::new(built.nodes.len(), built.edges.clone(), forces);
    for (i, node) in built.nodes.iter().enumerate() {
        layout.bodies[i].mass = 1.0 + (node.degree as f32).sqrt() * 0.5;
        if let Some((x, y)) = old.get(&node.id) {
            layout.bodies[i].x = *x;
            layout.bodies[i].y = *y;
        }
    }
    let mut steps = 0;
    while !layout.is_settled() && steps < MAX_STEPS {
        layout.step();
        steps += 1;
    }
    layout
}

/// A pane with nothing to paint: no vault, or no note open.
fn empty_pane(text: &str, theme: &Theme) -> gpui::AnyElement {
    div()
        .flex()
        .size_full()
        .min_h_0()
        .items_center()
        .justify_center()
        .px(px(12.))
        .py(px(12.))
        .text_size(px(12.))
        .text_color(theme.text_muted)
        .child(text.to_string())
        .into_any_element()
}

impl Render for LocalGraph {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.state.is_none() || self.current.is_none() {
            return empty_pane("Open a note to see its local graph", cx.global::<Theme>())
                .into_any_element();
        }
        if self.nodes.is_empty() {
            let theme = cx.global::<Theme>().clone();
            let text = if self.building {
                "Building graph…".to_string()
            } else {
                "This note has no links yet".to_string()
            };
            return empty_pane(&text, &theme).into_any_element();
        }
        let theme = cx.global::<Theme>().clone();
        let entity = cx.entity();
        let paint_entity = entity.clone();
        let node_count = self.nodes.len();
        let edge_count = self.edges.len();
        let line_mult = self.config.line_size_multiplier.unwrap_or(1.0);
        div()
            .key_context("LocalGraph")
            .track_focus(&self.focus_handle)
            .size_full()
            .min_h_0()
            .bg(theme.bg)
            .child(
                canvas(
                    move |bounds, window, cx| {
                        entity.update(cx, |view, _| {
                            // * Refit on every paint: the pane has no pan or
                            //   zoom to keep, so the whole graph is framed
                            //   from scratch, and a layout that settles
                            //   before the first paint would never be framed.
                            view.projection = view.projection_for(bounds);
                        });
                        window.insert_hitbox(bounds, HitboxBehavior::Normal)
                    },
                    move |bounds, hitbox, window, cx| {
                        let Some(projection) = paint_entity.read(cx).projection else {
                            return;
                        };
                        let theme = cx.global::<Theme>().clone();
                        let line_height = window.line_height();
                        let style = window.text_style();
                        let family = style.font().family.clone();
                        let (
                            nodes,
                            edges,
                            positions,
                            radii,
                            neighbors,
                            hovered,
                            current_ix,
                            label_cache,
                        ) = {
                            let view = paint_entity.read(cx);
                            let layout = view.layout.lock().unwrap_or_else(|e| e.into_inner());
                            let positions: Vec<Point<Pixels>> = layout
                                .bodies
                                .iter()
                                .map(|b| projection.to_screen(b.x, b.y))
                                .collect();
                            let radii: Vec<f32> =
                                view.nodes.iter().map(|n| view.node_radius(n)).collect();
                            let current_ix = view
                                .current
                                .as_ref()
                                .and_then(|c| view.index.get(c))
                                .copied();
                            (
                                view.nodes.clone(),
                                view.edges.clone(),
                                positions,
                                radii,
                                view.neighbors.clone(),
                                view.hovered,
                                current_ix,
                                view.labels.clone(),
                            )
                        };
                        // * The hovered node's neighbours stay bright, and
                        //   everything else fades: the one hop the pane is
                        //   of.
                        let highlight: Option<HashSet<usize>> = hovered.map(|h| {
                            let mut s: HashSet<usize> = neighbors
                                .get(h)
                                .cloned()
                                .unwrap_or_default()
                                .into_iter()
                                .collect();
                            s.insert(h);
                            s
                        });
                        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
                            // * Edges: one stroked path for the plain ones,
                            //   one for the highlighted pair.
                            let mut plain = PathBuilder::stroke(px(1.0 * line_mult));
                            let mut bright = PathBuilder::stroke(px(1.5 * line_mult));
                            let mut any_plain = false;
                            let mut any_bright = false;
                            for &(a, b) in edges.iter() {
                                let (pa, pb) = (positions[a as usize], positions[b as usize]);
                                if !bounds.contains(&pa) && !bounds.contains(&pb) {
                                    continue;
                                }
                                let on = hovered.is_some()
                                    && highlight
                                        .as_ref()
                                        .is_some_and(|s| s.contains(&(a as usize)))
                                    && highlight
                                        .as_ref()
                                        .is_some_and(|s| s.contains(&(b as usize)));
                                let line = if on {
                                    any_bright = true;
                                    &mut bright
                                } else {
                                    any_plain = true;
                                    &mut plain
                                };
                                line.move_to(pa);
                                line.line_to(pb);
                            }
                            let mut edge_color = theme.text_faint;
                            edge_color.a = if hovered.is_some() { 0.25 } else { 0.6 };
                            if any_plain {
                                if let Ok(path) = plain.build() {
                                    window.paint_path(path, edge_color);
                                }
                            }
                            if any_bright {
                                if let Ok(path) = bright.build() {
                                    window.paint_path(path, theme.accent);
                                }
                            }
                            // * Nodes, each with its label under it: a
                            //   depth-one graph is a handful of names, all of
                            //   which are worth reading.
                            for (i, node) in nodes.iter().enumerate() {
                                let p = positions[i];
                                let r = px(radii[i] * projection.scale.clamp(0.35, 1.6));
                                if p.x + r < bounds.left()
                                    || p.x - r > bounds.right()
                                    || p.y + r < bounds.top()
                                    || p.y - r > bounds.bottom()
                                {
                                    continue;
                                }
                                let dim = hovered.is_some()
                                    && !highlight.as_ref().is_some_and(|s| s.contains(&i));
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
                                let mut quad = fill(
                                    Bounds::new(point(p.x - r, p.y - r), gpui::size(d, d)),
                                    color,
                                );
                                quad.corner_radii = gpui::Corners::all(r);
                                if node.kind == NodeKind::Unresolved {
                                    quad.background = gpui::transparent_black().into();
                                    quad.border_widths = gpui::Edges::all(px(1.));
                                    quad.border_color = color;
                                }
                                window.paint_quad(quad);
                                let run_color = if dim { theme.text_faint } else { theme.text };
                                let key = (
                                    node.label.to_string(),
                                    [
                                        run_color.h.to_bits(),
                                        run_color.s.to_bits(),
                                        run_color.l.to_bits(),
                                        run_color.a.to_bits(),
                                    ],
                                    family.clone(),
                                );
                                let mut cache =
                                    label_cache.lock().unwrap_or_else(|e| e.into_inner());
                                let line = cache.entry(key).or_insert_with(|| {
                                    let mut run = style.to_run(node.label.len());
                                    run.color = run_color;
                                    window.text_system().shape_line(
                                        node.label.clone(),
                                        px(11.),
                                        &[run],
                                        None,
                                    )
                                });
                                let line: ShapedLine = line.clone();
                                drop(cache);
                                let origin = point(p.x - line.width / 2., p.y + r + px(3.));
                                let _ = line.paint(
                                    origin,
                                    line_height,
                                    gpui::TextAlign::Left,
                                    None,
                                    window,
                                    cx,
                                );
                            }
                        });

                        // ----- mouse -----------------------------------------
                        let e = paint_entity.clone();
                        let hb = hitbox.clone();
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                            if phase == gpui::DispatchPhase::Bubble && hb.is_hovered(window) {
                                e.update(cx, |v, cx| v.hover(event.position, cx));
                            }
                        });
                        let e = paint_entity.clone();
                        let hb = hitbox.clone();
                        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                            if phase == gpui::DispatchPhase::Bubble
                                && event.button == MouseButton::Left
                                && hb.is_hovered(window)
                            {
                                e.update(cx, |v, cx| v.click(event.position, cx));
                            }
                        });
                    },
                )
                .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .left_2()
                    .bottom_1()
                    .text_size(px(11.))
                    .text_color(theme.text_muted)
                    .child(format!("{node_count} notes, {edge_count} links")),
            )
            .into_any_element()
    }
}
