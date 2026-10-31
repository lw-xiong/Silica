//! The canvas view's model, without gpui: viewport math, the geometry of
//! cards and edges, hit testing, snapping, selection and the edit operations
//! with their undo history.
//!
//! The data is `silica_formats::canvas::Canvas`, which already reads and
//! writes the file in Obsidian's bytes; this crate is everything the view
//! needs to draw it and change it. Canvas space is the space of the file's
//! `x`, `y`, `width`, `height`; screen space is logical pixels of the view.
//!
//! Numbers marked `OBSERVE` are placeholders until the Obsidian probe
//! (`docs/silica/Specification.md`, "Observed canvas view behavior") pins
//! them down; change the constant, not the call sites.

use silica_formats::canvas::{Canvas, Edge, EndStyle, Node, NodeType, Side};

// ---- constants ---------------------------------------------------------------
// Observed in Obsidian 1.14.4 over CDP on 2026-10-21 unless marked otherwise;
// see `docs/silica/Specification.md`, "Observed canvas view behavior".

/// Snap grid in canvas units: the dot pattern's spacing at scale 1.
pub const GRID: f64 = 20.0;
/// How far one arrow key moves the selection, and with Shift held.
pub const NUDGE: f64 = GRID;
pub const NUDGE_LARGE: f64 = 5.0 * GRID;
/// Size of a text card made with a double click (centred on the pointer).
pub const DEFAULT_CARD: (f64, f64) = (250.0, 60.0);
/// Size of a file, link or group card made from the menus.
pub const DEFAULT_FILE_CARD: (f64, f64) = (400.0, 400.0);
/// Smallest a card can be resized to.
pub const MIN_NODE: (f64, f64) = (50.0, 50.0);
/// Zoom range as log2 of the scale, `Viewport::zoom`: scales 1/16 to 2.
pub const ZOOM_MIN: f64 = -4.0;
pub const ZOOM_MAX: f64 = 1.0;
/// At and below this zoom a card hides its content and shows a placeholder
/// (Obsidian's `mod-zoomed-out`, the "Zoom threshold" setting's default).
pub const ZOOM_THRESHOLD: f64 = -1.7;
/// Zoom change per wheel pixel with Ctrl held, in log2: a 100-pixel notch
/// is a third of a doubling.
pub const WHEEL_ZOOM: f64 = 1.0 / 300.0;
/// Arrow head length and base width in canvas units; the tip sits on the
/// card's edge.
pub const ARROW: f64 = 10.4;
pub const ARROW_WIDTH: f64 = 13.0;
/// The curve of an edge starts this far out from the card's edge. An end
/// without an arrow is joined to the card by a straight stub of this
/// length; an arrow covers it instead.
pub const EDGE_INSET: f64 = 7.0;
/// How far the Bézier control points sit from the curve's ends: half the
/// distance between those ends, kept within these bounds.
pub const EDGE_REACH: (f64, f64) = (70.0, 150.0);
/// Edge stroke width in canvas units at scale 1. Obsidian draws it as
/// `2 / sqrt(scale)` canvas units, so on screen it is `2 * sqrt(scale)`
/// pixels: thinner when zoomed out, but slower than the cards shrink.
pub const EDGE_WIDTH: f32 = 2.0;
/// Half the width of an edge's hit area in canvas units at scale 1; like
/// the stroke it scales as `1 / sqrt(scale)` in canvas units.
pub const EDGE_HIT: f64 = 12.0;
/// Card border width in canvas units: a 1-unit border plus a 1-unit inset
/// ring of the same colour.
pub const BORDER_WIDTH: f32 = 2.0;
/// Card corner radius in canvas units.
pub const CARD_RADIUS: f64 = 8.0;
/// Resize handle hit size in screen pixels: Obsidian's handles are 20-unit
/// strips along each side and 20-unit squares at the corners.
pub const HANDLE: f64 = 20.0;
/// How far from an edge's curve a click still hits it, screen pixels.
pub const EDGE_TOLERANCE: f64 = 12.0;

// ---- geometry -----------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Point {
        Point { x, y }
    }
    pub fn distance(self, other: Point) -> f64 {
        ((self.x - other.x).powi(2) + (self.y - other.y).powi(2)).sqrt()
    }
}

/// An axis-aligned rectangle in canvas units.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }
    pub fn of(node: &Node) -> Rect {
        Rect::new(node.x, node.y, node.width, node.height)
    }
    /// The rectangle of `width` by `height` centred on `c`: what a card
    /// created at the pointer or a view fitted around a point needs.
    pub fn from_center(c: Point, width: f64, height: f64) -> Rect {
        Rect::new(c.x - width / 2.0, c.y - height / 2.0, width, height)
    }
    /// The rectangle spanned by two corners, whichever way round.
    pub fn between(a: Point, b: Point) -> Rect {
        let x = a.x.min(b.x);
        let y = a.y.min(b.y);
        Rect::new(x, y, (a.x - b.x).abs(), (a.y - b.y).abs())
    }
    pub fn right(&self) -> f64 {
        self.x + self.width
    }
    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }
    pub fn center(&self) -> Point {
        Point::new(self.x + self.width / 2.0, self.y + self.height / 2.0)
    }
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x <= self.right() && p.y >= self.y && p.y <= self.bottom()
    }
    /// True when `other` lies entirely inside `self`.
    pub fn encloses(&self, other: &Rect) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.right() <= self.right()
            && other.bottom() <= self.bottom()
    }
    pub fn intersects(&self, other: &Rect) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }
    pub fn union(&self, other: &Rect) -> Rect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Rect::new(
            x,
            y,
            self.right().max(other.right()) - x,
            self.bottom().max(other.bottom()) - y,
        )
    }
    /// The overlap of two rectangles, `None` when they only touch or miss.
    pub fn intersection(&self, other: &Rect) -> Option<Rect> {
        if !self.intersects(other) {
            return None;
        }
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        Some(Rect::new(
            x,
            y,
            self.right().min(other.right()) - x,
            self.bottom().min(other.bottom()) - y,
        ))
    }
    pub fn translate(&self, dx: f64, dy: f64) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.width, self.height)
    }
    pub fn inflate(&self, by: f64) -> Rect {
        Rect::new(
            self.x - by,
            self.y - by,
            self.width + 2.0 * by,
            self.height + 2.0 * by,
        )
    }
}

/// The bounding box of some nodes, `None` for none.
pub fn bounds<'a>(nodes: impl IntoIterator<Item = &'a Node>) -> Option<Rect> {
    nodes.into_iter().map(Rect::of).reduce(|a, b| a.union(&b))
}

/// Indices of the nodes that show in `visible` (see [`Viewport::visible`]),
/// in z-order: what the view lays out and paints. A node touching the
/// edge of the view by a hair counts; a border or shadow may reach in.
pub fn visible_nodes(canvas: &Canvas, visible: &Rect) -> Vec<usize> {
    canvas
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| visible.intersects(&Rect::of(n)))
        .map(|(i, _)| i)
        .collect()
}

// ---- viewport -----------------------------------------------------------------

/// Where the view looks: the canvas point at the centre of the view and the
/// zoom as log2 of the scale, which is exactly what Obsidian records as
/// `viewState` (`x`, `y`, `zoom`) in `workspace.json` (observed: the
/// canvas transform is `translate(w/2 - x*2^zoom, h/2 - y*2^zoom) scale(2^zoom)`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
}

impl Default for Viewport {
    fn default() -> Viewport {
        Viewport {
            x: 0.0,
            y: 0.0,
            zoom: 0.0,
        }
    }
}

impl Viewport {
    /// Screen pixels per canvas unit.
    pub fn scale(&self) -> f64 {
        2f64.powf(self.zoom)
    }

    /// Canvas point to screen point in a view of `size` logical pixels.
    pub fn to_screen(&self, p: Point, size: (f64, f64)) -> Point {
        let s = self.scale();
        Point::new(
            (p.x - self.x) * s + size.0 / 2.0,
            (p.y - self.y) * s + size.1 / 2.0,
        )
    }

    /// Screen point back to canvas space.
    pub fn to_canvas(&self, p: Point, size: (f64, f64)) -> Point {
        let s = self.scale();
        Point::new(
            (p.x - size.0 / 2.0) / s + self.x,
            (p.y - size.1 / 2.0) / s + self.y,
        )
    }

    /// Canvas rectangle to screen rectangle.
    pub fn rect_to_screen(&self, r: Rect, size: (f64, f64)) -> Rect {
        let s = self.scale();
        let o = self.to_screen(Point::new(r.x, r.y), size);
        Rect::new(o.x, o.y, r.width * s, r.height * s)
    }

    /// The canvas rectangle the view shows.
    pub fn visible(&self, size: (f64, f64)) -> Rect {
        let a = self.to_canvas(Point::new(0.0, 0.0), size);
        let b = self.to_canvas(Point::new(size.0, size.1), size);
        Rect::between(a, b)
    }

    /// Moves the view by a screen-space delta (the content follows the pointer).
    pub fn pan(&mut self, dx: f64, dy: f64) {
        let s = self.scale();
        self.x -= dx / s;
        self.y -= dy / s;
    }

    /// Changes the zoom by `delta` (log2) keeping the canvas point under the
    /// screen point `at` fixed, clamped to the zoom range.
    pub fn zoom_at(&mut self, delta: f64, at: Point, size: (f64, f64)) {
        let before = self.to_canvas(at, size);
        self.zoom = (self.zoom + delta).clamp(ZOOM_MIN, ZOOM_MAX);
        let after = self.to_canvas(at, size);
        self.x += before.x - after.x;
        self.y += before.y - after.y;
    }

    /// Frames `rect` in a view of `size` with `padding` screen pixels around it.
    pub fn fit(rect: Rect, size: (f64, f64), padding: f64) -> Viewport {
        let avail = (
            (size.0 - 2.0 * padding).max(1.0),
            (size.1 - 2.0 * padding).max(1.0),
        );
        let scale = (avail.0 / rect.width.max(1.0)).min(avail.1 / rect.height.max(1.0));
        let c = rect.center();
        Viewport {
            x: c.x,
            y: c.y,
            zoom: scale.log2().clamp(ZOOM_MIN, ZOOM_MAX),
        }
    }

    /// Frames some nodes the way `fit` frames a rectangle: Obsidian's
    /// "zoom to fit" over the whole canvas or "zoom to selection" over the
    /// selected nodes. `None` when there is nothing to frame, so the caller
    /// decides what an empty canvas shows (usually [`Viewport::default`]).
    pub fn fit_nodes<'a>(
        nodes: impl IntoIterator<Item = &'a Node>,
        size: (f64, f64),
        padding: f64,
    ) -> Option<Viewport> {
        bounds(nodes).map(|r| Viewport::fit(r, size, padding))
    }

    /// Back to the origin at scale 1, Obsidian's "zoom to 100%" with the
    /// view centred where a new canvas starts.
    pub fn reset(&mut self) {
        *self = Viewport::default();
    }

    /// Changes the zoom by `delta` (log2) around the centre of the view: the
    /// zoom-in and zoom-out commands, which have no pointer to zoom around.
    pub fn zoom_step(&mut self, delta: f64, size: (f64, f64)) {
        let centre = Point::new(size.0 / 2.0, size.1 / 2.0);
        self.zoom_at(delta, centre, size);
    }
}

// ---- colours -----------------------------------------------------------------

/// What a node's or edge's `color` means: one of Obsidian's six presets,
/// a hex colour, or the theme's default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Default,
    /// `"1"` to `"6"`: red, orange, yellow, green, cyan, purple.
    Preset(u8),
    /// `"#rrggbb"` as `0xrrggbb`.
    Hex(u32),
}

impl Color {
    pub fn parse(color: Option<&str>) -> Color {
        match color {
            None | Some("") => Color::Default,
            Some(s) => {
                if let Some(hex) = s.strip_prefix('#') {
                    if hex.len() == 6 {
                        if let Ok(v) = u32::from_str_radix(hex, 16) {
                            return Color::Hex(v);
                        }
                    }
                    Color::Default
                } else {
                    match s.parse::<u8>() {
                        Ok(n @ 1..=6) => Color::Preset(n),
                        _ => Color::Default,
                    }
                }
            }
        }
    }

    /// The string the file carries for this colour, `None` for the default.
    pub fn to_file(self) -> Option<String> {
        match self {
            Color::Default => None,
            Color::Preset(n) => Some(n.to_string()),
            Color::Hex(v) => Some(format!("#{v:06x}")),
        }
    }
}

// ---- edges ---------------------------------------------------------------------

/// A cubic Bézier from one card side to another.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgePath {
    pub from: Point,
    pub c1: Point,
    pub c2: Point,
    pub to: Point,
}

impl EdgePath {
    pub fn point_at(&self, t: f64) -> Point {
        let u = 1.0 - t;
        let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        Point::new(
            a * self.from.x + b * self.c1.x + c * self.c2.x + d * self.to.x,
            a * self.from.y + b * self.c1.y + c * self.c2.y + d * self.to.y,
        )
    }

    /// `segments + 1` points along the curve, from start to end; at least
    /// the two ends, so zero segments is the chord.
    pub fn flatten(&self, segments: usize) -> Vec<Point> {
        let segments = segments.max(1);
        (0..=segments)
            .map(|i| self.point_at(i as f64 / segments as f64))
            .collect()
    }

    /// Shortest distance from `p` to the curve, sampled.
    pub fn distance_to(&self, p: Point) -> f64 {
        self.flatten(32)
            .windows(2)
            .map(|w| segment_distance(w[0], w[1], p))
            .fold(f64::INFINITY, f64::min)
    }

    /// The unit direction of travel at `t`. The arrow head at `to` points
    /// along `tangent_at(1.0)`; one at `from` points against
    /// `tangent_at(0.0)`. A degenerate curve (control points on their
    /// anchors, or all four points together) falls back to the chord, and
    /// a curve with no extent at all to pointing right, so a head is always
    /// drawable.
    pub fn tangent_at(&self, t: f64) -> Point {
        let u = 1.0 - t;
        let (a, b, c) = (3.0 * u * u, 6.0 * u * t, 3.0 * t * t);
        let d = Point::new(
            a * (self.c1.x - self.from.x)
                + b * (self.c2.x - self.c1.x)
                + c * (self.to.x - self.c2.x),
            a * (self.c1.y - self.from.y)
                + b * (self.c2.y - self.c1.y)
                + c * (self.to.y - self.c2.y),
        );
        unit(d)
            .or_else(|| unit(Point::new(self.to.x - self.from.x, self.to.y - self.from.y)))
            .unwrap_or(Point::new(1.0, 0.0))
    }

    /// A box that contains the whole curve: the control polygon's, which
    /// the curve never leaves. Slightly generous, which is what culling and
    /// a dirty region want.
    pub fn bounds(&self) -> Rect {
        [self.c1, self.c2, self.to]
            .iter()
            .fold(Rect::new(self.from.x, self.from.y, 0.0, 0.0), |r, p| {
                r.union(&Rect::new(p.x, p.y, 0.0, 0.0))
            })
    }

    /// Where a label sits: the curve's parametric middle, which for the
    /// symmetric curves `edge_path` builds is also its visual middle.
    pub fn midpoint(&self) -> Point {
        self.point_at(0.5)
    }

    /// The curve's length, sampled like `distance_to`.
    pub fn length(&self) -> f64 {
        self.flatten(32)
            .windows(2)
            .map(|w| w[0].distance(w[1]))
            .sum()
    }
}

/// `v` scaled to length 1, `None` for the zero vector.
fn unit(v: Point) -> Option<Point> {
    let len = (v.x * v.x + v.y * v.y).sqrt();
    (len > f64::EPSILON).then(|| Point::new(v.x / len, v.y / len))
}

fn segment_distance(a: Point, b: Point, p: Point) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0)
    };
    p.distance(Point::new(a.x + t * dx, a.y + t * dy))
}

/// The unit vector pointing out of a side.
pub fn side_normal(side: Side) -> Point {
    match side {
        Side::Top => Point::new(0.0, -1.0),
        Side::Right => Point::new(1.0, 0.0),
        Side::Bottom => Point::new(0.0, 1.0),
        Side::Left => Point::new(-1.0, 0.0),
    }
}

/// The middle of a side of a rectangle.
pub fn anchor(rect: &Rect, side: Side) -> Point {
    match side {
        Side::Top => Point::new(rect.x + rect.width / 2.0, rect.y),
        Side::Right => Point::new(rect.right(), rect.y + rect.height / 2.0),
        Side::Bottom => Point::new(rect.x + rect.width / 2.0, rect.bottom()),
        Side::Left => Point::new(rect.x, rect.y + rect.height / 2.0),
    }
}

/// The side of `rect` that faces `towards` — what an edge without a recorded
/// side uses, and what a dragged edge end snaps to.
pub fn facing_side(rect: &Rect, towards: Point) -> Side {
    let c = rect.center();
    let (dx, dy) = (towards.x - c.x, towards.y - c.y);
    if dx.abs() * rect.height >= dy.abs() * rect.width {
        if dx >= 0.0 {
            Side::Right
        } else {
            Side::Left
        }
    } else if dy >= 0.0 {
        Side::Bottom
    } else {
        Side::Top
    }
}

/// The curve of an edge between two cards: from `EDGE_INSET` outside the
/// middle of one side to the same distance outside the other, leaving and
/// arriving along the sides' normals.
pub fn edge_path(from: &Rect, from_side: Side, to: &Rect, to_side: Side) -> EdgePath {
    let a = anchor(from, from_side);
    let b = anchor(to, to_side);
    edge_path_points(a, from_side, b, to_side)
}

/// The curve between two anchors on the cards' edges, for a drawn edge
/// whose end is a free point too. The curve itself runs between the points
/// `EDGE_INSET` out from each anchor along its side's normal, with the
/// control points `EDGE_REACH` further out.
pub fn edge_path_points(a: Point, from_side: Side, b: Point, to_side: Side) -> EdgePath {
    let na = side_normal(from_side);
    let nb = side_normal(to_side);
    let from = Point::new(a.x + na.x * EDGE_INSET, a.y + na.y * EDGE_INSET);
    let to = Point::new(b.x + nb.x * EDGE_INSET, b.y + nb.y * EDGE_INSET);
    let reach = (from.distance(to) * 0.5).clamp(EDGE_REACH.0, EDGE_REACH.1);
    EdgePath {
        from,
        c1: Point::new(from.x + na.x * reach, from.y + na.y * reach),
        c2: Point::new(to.x + nb.x * reach, to.y + nb.y * reach),
        to,
    }
}

/// The straight stub between a card's edge and the start of the curve,
/// drawn when that end carries no arrow.
pub fn edge_stub(anchor: Point, side: Side) -> (Point, Point) {
    let n = side_normal(side);
    (
        anchor,
        Point::new(anchor.x + n.x * EDGE_INSET, anchor.y + n.y * EDGE_INSET),
    )
}

/// The three corners of an arrow head whose tip is `tip` and which points
/// along `direction` (a unit vector), `length` long and `ARROW_WIDTH` wide
/// at `ARROW` length (the proportions are kept for other lengths).
pub fn arrow_head(tip: Point, direction: Point, length: f64) -> [Point; 3] {
    let half = length * ARROW_WIDTH / ARROW / 2.0;
    let base = Point::new(tip.x - direction.x * length, tip.y - direction.y * length);
    let perp = Point::new(-direction.y, direction.x);
    [
        tip,
        Point::new(base.x + perp.x * half, base.y + perp.y * half),
        Point::new(base.x - perp.x * half, base.y - perp.y * half),
    ]
}

/// Which ends of an edge carry an arrow, with Obsidian's defaults applied.
pub fn edge_ends(edge: &Edge) -> (bool, bool) {
    let from = edge.from_end == Some(EndStyle::Arrow);
    let to = edge.to_end != Some(EndStyle::None);
    (from, to)
}

// ---- hit testing --------------------------------------------------------------

/// Topmost node under `p` in canvas space: later in the array is above.
/// A group only counts by its body when nothing else is hit; cards win.
pub fn hit_node(canvas: &Canvas, p: Point) -> Option<usize> {
    let card = canvas
        .nodes
        .iter()
        .enumerate()
        .rev()
        .find(|(_, n)| n.kind != NodeType::Group && Rect::of(n).contains(p))
        .map(|(i, _)| i);
    card.or_else(|| {
        canvas
            .nodes
            .iter()
            .enumerate()
            .rev()
            .find(|(_, n)| n.kind == NodeType::Group && Rect::of(n).contains(p))
            .map(|(i, _)| i)
    })
}

/// The edge whose curve passes within `tolerance` canvas units of `p`; the
/// nearest when several do. [`EDGE_TOLERANCE`] is in screen pixels, so pass
/// `EDGE_TOLERANCE / viewport.scale()`.
pub fn hit_edge(canvas: &Canvas, p: Point, tolerance: f64) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (i, edge) in canvas.edges.iter().enumerate() {
        let Some(path) = resolved_edge_path(canvas, edge) else {
            continue;
        };
        let d = path.distance_to(p);
        if d <= tolerance && best.is_none_or(|(_, bd)| d < bd) {
            best = Some((i, d));
        }
    }
    best.map(|(i, _)| i)
}

/// The curve of an edge whose ends both resolve to nodes.
pub fn resolved_edge_path(canvas: &Canvas, edge: &Edge) -> Option<EdgePath> {
    let from = canvas.nodes.iter().find(|n| n.id == edge.from_node)?;
    let to = canvas.nodes.iter().find(|n| n.id == edge.to_node)?;
    let (fr, tr) = (Rect::of(from), Rect::of(to));
    let from_side = edge
        .from_side
        .unwrap_or_else(|| facing_side(&fr, tr.center()));
    let to_side = edge
        .to_side
        .unwrap_or_else(|| facing_side(&tr, fr.center()));
    Some(edge_path(&fr, from_side, &tr, to_side))
}

/// Indices of the nodes a marquee selects: those its rectangle intersects
/// (observed: a card half covered is selected).
pub fn nodes_in(canvas: &Canvas, marquee: &Rect) -> Vec<usize> {
    canvas
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| marquee.intersects(&Rect::of(n)))
        .map(|(i, _)| i)
        .collect()
}

/// Indices of the nodes wholly inside a group, which move with it. A nested
/// group that fits inside counts too, so moving the outer one carries the
/// whole stack; the group itself does not.
pub fn group_members(canvas: &Canvas, group: usize) -> Vec<usize> {
    let Some(g) = canvas.nodes.get(group) else {
        return Vec::new();
    };
    let gr = Rect::of(g);
    canvas
        .nodes
        .iter()
        .enumerate()
        .filter(|(i, n)| *i != group && gr.encloses(&Rect::of(n)))
        .map(|(i, _)| i)
        .collect()
}

// ---- resize handles -------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Handle {
    Top,
    Right,
    Bottom,
    Left,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Handle {
    pub const ALL: [Handle; 8] = [
        Handle::Top,
        Handle::Right,
        Handle::Bottom,
        Handle::Left,
        Handle::TopLeft,
        Handle::TopRight,
        Handle::BottomLeft,
        Handle::BottomRight,
    ];

    /// The handle's point on a rectangle.
    pub fn position(self, r: &Rect) -> Point {
        let (cx, cy) = (r.x + r.width / 2.0, r.y + r.height / 2.0);
        match self {
            Handle::Top => Point::new(cx, r.y),
            Handle::Right => Point::new(r.right(), cy),
            Handle::Bottom => Point::new(cx, r.bottom()),
            Handle::Left => Point::new(r.x, cy),
            Handle::TopLeft => Point::new(r.x, r.y),
            Handle::TopRight => Point::new(r.right(), r.y),
            Handle::BottomLeft => Point::new(r.x, r.bottom()),
            Handle::BottomRight => Point::new(r.right(), r.bottom()),
        }
    }

    /// Moves the handle's side(s) by a canvas-space delta, keeping the
    /// opposite side still and the rectangle at least `min`.
    pub fn resize(self, r: &Rect, dx: f64, dy: f64, min: (f64, f64)) -> Rect {
        let mut out = *r;
        let moves_left = matches!(self, Handle::Left | Handle::TopLeft | Handle::BottomLeft);
        let moves_right = matches!(self, Handle::Right | Handle::TopRight | Handle::BottomRight);
        let moves_top = matches!(self, Handle::Top | Handle::TopLeft | Handle::TopRight);
        let moves_bottom = matches!(
            self,
            Handle::Bottom | Handle::BottomLeft | Handle::BottomRight
        );
        if moves_left {
            let nx = (r.x + dx).min(r.right() - min.0);
            out.width = r.right() - nx;
            out.x = nx;
        }
        if moves_right {
            out.width = (r.width + dx).max(min.0);
        }
        if moves_top {
            let ny = (r.y + dy).min(r.bottom() - min.1);
            out.height = r.bottom() - ny;
            out.y = ny;
        }
        if moves_bottom {
            out.height = (r.height + dy).max(min.1);
        }
        out
    }
}

/// The handle under a screen point for a rectangle in screen space, if any.
pub fn hit_handle(screen_rect: &Rect, p: Point, size: f64) -> Option<Handle> {
    Handle::ALL.into_iter().find(|h| {
        let c = h.position(screen_rect);
        (p.x - c.x).abs() <= size / 2.0 && (p.y - c.y).abs() <= size / 2.0
    })
}

// ---- snapping and ids -----------------------------------------------------------

/// Rounds to the grid.
pub fn snap(v: f64) -> f64 {
    (v / GRID).round() * GRID
}

pub fn snap_point(p: Point) -> Point {
    Point::new(snap(p.x), snap(p.y))
}

/// A fresh 16-hex-digit id like Obsidian's.
pub fn new_id() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut h = RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    );
    format!("{:016x}", h.finish())
}

// ---- selection ----------------------------------------------------------------

/// What is selected, by id so it survives reordering and reloads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    pub nodes: Vec<String>,
    pub edges: Vec<String>,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.edges.is_empty()
    }
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.edges.clear();
    }
    pub fn only_node(id: &str) -> Selection {
        Selection {
            nodes: vec![id.to_string()],
            edges: Vec::new(),
        }
    }
    pub fn only_edge(id: &str) -> Selection {
        Selection {
            nodes: Vec::new(),
            edges: vec![id.to_string()],
        }
    }
    pub fn has_node(&self, id: &str) -> bool {
        self.nodes.iter().any(|n| n == id)
    }
    pub fn has_edge(&self, id: &str) -> bool {
        self.edges.iter().any(|e| e == id)
    }
    /// Adds or removes a node, as a shift-click does.
    pub fn toggle_node(&mut self, id: &str) {
        if let Some(i) = self.nodes.iter().position(|n| n == id) {
            self.nodes.remove(i);
        } else {
            self.nodes.push(id.to_string());
        }
    }
    pub fn toggle_edge(&mut self, id: &str) {
        if let Some(i) = self.edges.iter().position(|e| e == id) {
            self.edges.remove(i);
        } else {
            self.edges.push(id.to_string());
        }
    }
    /// Drops ids the canvas no longer has.
    pub fn retain_present(&mut self, canvas: &Canvas) {
        self.nodes
            .retain(|id| canvas.nodes.iter().any(|n| &n.id == id));
        self.edges
            .retain(|id| canvas.edges.iter().any(|e| &e.id == id));
    }
}

// ---- edit operations with history ------------------------------------------------

/// The canvas being edited: the data, what is selected, and an undo history
/// of whole snapshots (a canvas is small; this keeps every operation one
/// `commit`). Drags call the mutators every frame and `commit` once, at the
/// end, so one drag is one undo step.
#[derive(Debug, Clone)]
pub struct Scene {
    pub canvas: Canvas,
    pub selection: Selection,
    undo: Vec<Canvas>,
    redo: Vec<Canvas>,
    /// The state the last `commit` started from, set by `begin`.
    pending: Option<Canvas>,
    /// True when the canvas differs from what was last saved.
    dirty: bool,
}

impl Scene {
    pub fn new(canvas: Canvas) -> Scene {
        Scene {
            canvas,
            selection: Selection::default(),
            undo: Vec::new(),
            redo: Vec::new(),
            pending: None,
            dirty: false,
        }
    }

    /// The file changed on disk: take the new data, keep what selection still applies.
    pub fn reload(&mut self, canvas: Canvas) {
        self.canvas = canvas;
        self.selection.retain_present(&self.canvas);
        self.undo.clear();
        self.redo.clear();
        self.pending = None;
        self.dirty = false;
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn mark_saved(&mut self) {
        self.dirty = false;
    }

    /// Marks the canvas as needing a save without closing an undo step:
    /// for a text card being typed into, which saves while the session's
    /// single step stays open.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Marks the start of an operation; `commit` records the step. Calling
    /// `begin` twice without a `commit` keeps the first snapshot.
    pub fn begin(&mut self) {
        if self.pending.is_none() {
            self.pending = Some(self.canvas.clone());
        }
    }

    /// Records the operation since `begin` as one undo step, if it changed
    /// anything.
    pub fn commit(&mut self) {
        if let Some(before) = self.pending.take() {
            if before != self.canvas {
                self.undo.push(before);
                self.redo.clear();
                self.dirty = true;
            }
        }
    }

    /// `begin`, the closure, `commit`.
    pub fn edit(&mut self, f: impl FnOnce(&mut Canvas)) {
        self.begin();
        f(&mut self.canvas);
        self.commit();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// How many steps `undo` has to give: what a test counts, and what a
    /// history menu shows.
    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }

    pub fn undo(&mut self) -> bool {
        let Some(before) = self.undo.pop() else {
            return false;
        };
        self.pending = None;
        self.redo.push(std::mem::replace(&mut self.canvas, before));
        self.selection.retain_present(&self.canvas);
        self.dirty = true;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(after) = self.redo.pop() else {
            return false;
        };
        self.pending = None;
        self.undo.push(std::mem::replace(&mut self.canvas, after));
        self.selection.retain_present(&self.canvas);
        self.dirty = true;
        true
    }

    // ---- lookups ----

    pub fn node_index(&self, id: &str) -> Option<usize> {
        self.canvas.nodes.iter().position(|n| n.id == id)
    }

    pub fn node(&self, id: &str) -> Option<&Node> {
        self.canvas.nodes.iter().find(|n| n.id == id)
    }

    pub fn node_mut(&mut self, id: &str) -> Option<&mut Node> {
        self.canvas.nodes.iter_mut().find(|n| n.id == id)
    }

    pub fn edge(&self, id: &str) -> Option<&Edge> {
        self.canvas.edges.iter().find(|e| e.id == id)
    }

    pub fn edge_mut(&mut self, id: &str) -> Option<&mut Edge> {
        self.canvas.edges.iter_mut().find(|e| e.id == id)
    }

    /// Bounding box of the selected nodes.
    pub fn selection_bounds(&self) -> Option<Rect> {
        bounds(self.selection.nodes.iter().filter_map(|id| self.node(id)))
    }

    // ---- mutators (call between `begin` and `commit`) ----

    /// Adds a node on top and selects it; returns its id.
    pub fn add_node(&mut self, node: Node) -> String {
        let id = node.id.clone();
        self.canvas.nodes.push(node);
        self.selection = Selection::only_node(&id);
        id
    }

    /// A text card of the default size with its top-left at `at`.
    pub fn add_text_card(&mut self, at: Point, text: &str) -> String {
        let mut node = Node::new(
            new_id(),
            NodeType::Text,
            at.x,
            at.y,
            DEFAULT_CARD.0,
            DEFAULT_CARD.1,
        );
        node.text = Some(text.to_string());
        self.add_node(node)
    }

    /// A card embedding a vault file.
    pub fn add_file_card(&mut self, at: Point, file: &str, subpath: Option<&str>) -> String {
        let mut node = Node::new(
            new_id(),
            NodeType::File,
            at.x,
            at.y,
            DEFAULT_FILE_CARD.0,
            DEFAULT_FILE_CARD.1,
        );
        node.file = Some(file.to_string());
        node.subpath = subpath.map(str::to_string);
        self.add_node(node)
    }

    /// A card showing a web page.
    pub fn add_link_card(&mut self, at: Point, url: &str) -> String {
        let mut node = Node::new(
            new_id(),
            NodeType::Link,
            at.x,
            at.y,
            DEFAULT_FILE_CARD.0,
            DEFAULT_FILE_CARD.1,
        );
        node.url = Some(url.to_string());
        self.add_node(node)
    }

    /// A group around `rect`, inserted beneath every card the way Obsidian
    /// orders groups first.
    pub fn add_group(&mut self, rect: Rect, label: Option<&str>) -> String {
        let mut node = Node::new(
            new_id(),
            NodeType::Group,
            rect.x,
            rect.y,
            rect.width,
            rect.height,
        );
        node.label = Some(label.unwrap_or("Untitled group").to_string());
        let id = node.id.clone();
        let first_card = self
            .canvas
            .nodes
            .iter()
            .position(|n| n.kind != NodeType::Group)
            .unwrap_or(self.canvas.nodes.len());
        self.canvas.nodes.insert(first_card, node);
        self.selection = Selection::only_node(&id);
        id
    }

    /// Adds an edge and selects it; returns its id.
    pub fn add_edge(&mut self, from: &str, from_side: Side, to: &str, to_side: Side) -> String {
        let mut edge = Edge::new(new_id(), from, to);
        edge.from_side = Some(from_side);
        edge.to_side = Some(to_side);
        let id = edge.id.clone();
        self.canvas.edges.push(edge);
        self.selection = Selection::only_edge(&id);
        id
    }

    /// The nodes an operation on the selection carries along: the selected
    /// nodes and the members of every selected group, each once, in z-order.
    /// Moving, copying and duplicating all take this set, so a group goes
    /// with its contents the way it does in Obsidian.
    pub fn carried_nodes(&self) -> Vec<usize> {
        let mut carried = vec![false; self.canvas.nodes.len()];
        for id in &self.selection.nodes {
            if let Some(ix) = self.node_index(id) {
                carried[ix] = true;
                if self.canvas.nodes[ix].kind == NodeType::Group {
                    for m in group_members(&self.canvas, ix) {
                        carried[m] = true;
                    }
                }
            }
        }
        (0..carried.len()).filter(|&i| carried[i]).collect()
    }

    /// Moves the selected nodes (and the members of selected groups) by a
    /// canvas-space delta.
    pub fn move_selection(&mut self, dx: f64, dy: f64) {
        for ix in self.carried_nodes() {
            let n = &mut self.canvas.nodes[ix];
            n.x += dx;
            n.y += dy;
        }
    }

    /// Moves the selection by a delta and lands every carried node on the
    /// grid: what the arrow keys do, one grid step at a time.
    pub fn nudge_selection(&mut self, dx: f64, dy: f64) {
        for ix in self.carried_nodes() {
            let n = &mut self.canvas.nodes[ix];
            n.x = snap(n.x + dx);
            n.y = snap(n.y + dy);
        }
    }

    /// Puts one node exactly at `rect`: the end of a resize that was shown
    /// live, or an alignment command.
    pub fn set_node_rect(&mut self, id: &str, rect: Rect) {
        if let Some(n) = self.node_mut(id) {
            n.x = rect.x;
            n.y = rect.y;
            n.width = rect.width;
            n.height = rect.height;
        }
    }

    /// Resizes one node by a handle.
    pub fn resize_node(&mut self, id: &str, handle: Handle, dx: f64, dy: f64) {
        if let Some(n) = self.node_mut(id) {
            let r = handle.resize(&Rect::of(n), dx, dy, MIN_NODE);
            n.x = r.x;
            n.y = r.y;
            n.width = r.width;
            n.height = r.height;
        }
    }

    /// Snaps the selected nodes' positions to the grid.
    pub fn snap_selection(&mut self) {
        for id in self.selection.nodes.clone() {
            if let Some(n) = self.node_mut(&id) {
                n.x = snap(n.x);
                n.y = snap(n.y);
            }
        }
    }

    /// Removes the selected nodes and edges, and the edges that touched a
    /// removed node.
    pub fn delete_selection(&mut self) {
        let nodes = std::mem::take(&mut self.selection.nodes);
        let edges = std::mem::take(&mut self.selection.edges);
        self.canvas.nodes.retain(|n| !nodes.contains(&n.id));
        self.canvas.edges.retain(|e| {
            !edges.contains(&e.id) && !nodes.contains(&e.from_node) && !nodes.contains(&e.to_node)
        });
    }

    /// Sets the colour of everything selected.
    pub fn set_selection_color(&mut self, color: Color) {
        let value = color.to_file();
        for id in self.selection.nodes.clone() {
            if let Some(n) = self.node_mut(&id) {
                n.color = value.clone();
            }
        }
        for id in self.selection.edges.clone() {
            if let Some(e) = self.edge_mut(&id) {
                e.color = value.clone();
            }
        }
    }

    /// Replaces a text card's text.
    pub fn set_text(&mut self, id: &str, text: &str) {
        if let Some(n) = self.node_mut(id) {
            n.text = Some(text.to_string());
        }
    }

    /// Sets an edge's label; empty removes it.
    pub fn set_edge_label(&mut self, id: &str, label: &str) {
        if let Some(e) = self.edge_mut(id) {
            e.label = (!label.is_empty()).then(|| label.to_string());
        }
    }

    /// Sets a group's label; empty removes it.
    pub fn set_group_label(&mut self, id: &str, label: &str) {
        if let Some(n) = self.node_mut(id) {
            n.label = (!label.is_empty()).then(|| label.to_string());
        }
    }

    /// Sets which ends of an edge carry an arrow.
    pub fn set_edge_ends(&mut self, id: &str, from_arrow: bool, to_arrow: bool) {
        if let Some(e) = self.edge_mut(id) {
            e.from_end = from_arrow.then_some(EndStyle::Arrow);
            e.to_end = (!to_arrow).then_some(EndStyle::None);
        }
    }

    /// Reverses an edge's direction.
    pub fn flip_edge(&mut self, id: &str) {
        if let Some(e) = self.edge_mut(id) {
            std::mem::swap(&mut e.from_node, &mut e.to_node);
            std::mem::swap(&mut e.from_side, &mut e.to_side);
        }
    }

    /// Selects everything.
    pub fn select_all(&mut self) {
        self.selection.nodes = self.canvas.nodes.iter().map(|n| n.id.clone()).collect();
        self.selection.edges = self.canvas.edges.iter().map(|e| e.id.clone()).collect();
    }

    /// Brings the selected nodes to the top of the z-order, keeping their
    /// relative order.
    pub fn raise_selection(&mut self) {
        let ids = self.selection.nodes.clone();
        let (mut top, rest): (Vec<Node>, Vec<Node>) = self
            .canvas
            .nodes
            .drain(..)
            .partition(|n| ids.contains(&n.id) && n.kind != NodeType::Group);
        self.canvas.nodes = rest;
        self.canvas.nodes.append(&mut top);
    }

    /// Grows or shrinks a group to its members' bounding box plus `padding`
    /// on every side. Members are what the group encloses now, so a group
    /// that has drifted off a card does not snap back onto it; nothing
    /// happens to an empty group.
    pub fn align_group_to_members(&mut self, group_id: &str, padding: f64) {
        let Some(ix) = self.node_index(group_id) else {
            return;
        };
        if self.canvas.nodes[ix].kind != NodeType::Group {
            return;
        }
        let members = group_members(&self.canvas, ix);
        let Some(r) = bounds(members.iter().map(|&m| &self.canvas.nodes[m])) else {
            return;
        };
        self.set_node_rect(group_id, r.inflate(padding));
    }

    /// Obsidian's "group selection" (`Ctrl+G`): a new group `padding` around
    /// the selected nodes, which becomes the selection. Edges alone cannot
    /// be grouped, so `None` when no node is selected.
    pub fn group_selection(&mut self, padding: f64) -> Option<String> {
        let r = self.selection_bounds()?;
        Some(self.add_group(r.inflate(padding), None))
    }

    /// The selected nodes (with the members of selected groups) and the
    /// edges that join two of them, as a canvas of their own with the ids
    /// kept: `copy_selection().to_json()` is the clipboard text, which
    /// `Canvas::from_json` reads back for [`Scene::paste`]. Nodes keep their
    /// z-order. Empty when nothing is selected.
    pub fn copy_selection(&self) -> Canvas {
        let nodes: Vec<Node> = self
            .carried_nodes()
            .into_iter()
            .map(|ix| self.canvas.nodes[ix].clone())
            .collect();
        let edges = self
            .canvas
            .edges
            .iter()
            .filter(|e| {
                nodes.iter().any(|n| n.id == e.from_node) && nodes.iter().any(|n| n.id == e.to_node)
            })
            .cloned()
            .collect();
        let mut clip = Canvas::default();
        clip.nodes = nodes;
        clip.edges = edges;
        clip
    }

    /// Adds a copy of `clip` with fresh ids, the edges remapped onto them
    /// (one whose end is not in the clip is dropped), placed so the clip's
    /// bounding box has its top-left at `at`, or one grid step down and right
    /// of where it was when `at` is `None` — paste at the pointer, or paste
    /// in place. The pasted nodes become the selection; their ids come back
    /// in clip order.
    pub fn paste(&mut self, clip: &Canvas, at: Option<Point>) -> Vec<String> {
        let Some(r) = bounds(&clip.nodes) else {
            return Vec::new();
        };
        let (dx, dy) = match at {
            Some(p) => (p.x - r.x, p.y - r.y),
            None => (GRID, GRID),
        };
        self.paste_offset(clip, dx, dy)
    }

    /// Copies the selected nodes (with the members of selected groups) and
    /// the edges between them, moved by `offset`, and selects the copies.
    /// Their ids come back in z-order. Obsidian's duplicate is this with a
    /// grid step.
    pub fn duplicate_selection(&mut self, offset: Point) -> Vec<String> {
        let clip = self.copy_selection();
        self.paste_offset(&clip, offset.x, offset.y)
    }

    fn paste_offset(&mut self, clip: &Canvas, dx: f64, dy: f64) -> Vec<String> {
        let ids: Vec<(String, String)> = clip
            .nodes
            .iter()
            .map(|n| (n.id.clone(), new_id()))
            .collect();
        let renamed = |old: &str| ids.iter().find(|(o, _)| o == old).map(|(_, n)| n.clone());
        for (node, (_, id)) in clip.nodes.iter().zip(&ids) {
            let mut copy = node.clone();
            copy.id = id.clone();
            copy.x += dx;
            copy.y += dy;
            self.canvas.nodes.push(copy);
        }
        for edge in &clip.edges {
            let (Some(from), Some(to)) = (renamed(&edge.from_node), renamed(&edge.to_node)) else {
                continue;
            };
            let mut copy = edge.clone();
            copy.id = new_id();
            copy.from_node = from;
            copy.to_node = to;
            self.canvas.edges.push(copy);
        }
        let new_ids: Vec<String> = ids.into_iter().map(|(_, n)| n).collect();
        self.selection = Selection {
            nodes: new_ids.clone(),
            edges: Vec::new(),
        };
        new_ids
    }

    // ---- lookups over the structure ----

    /// The edges touching a node at either end, in file order.
    pub fn edges_of(&self, node_id: &str) -> Vec<&Edge> {
        self.canvas
            .edges
            .iter()
            .filter(|e| e.from_node == node_id || e.to_node == node_id)
            .collect()
    }

    /// The groups a user can jump to by name, in z-order: the labelled ones,
    /// which is what Obsidian's `canvas:jump-to-group` lists.
    pub fn jump_targets(&self) -> Vec<&Node> {
        self.canvas
            .nodes
            .iter()
            .filter(|n| {
                n.kind == NodeType::Group && n.label.as_deref().is_some_and(|l| !l.is_empty())
            })
            .collect()
    }

    /// The file's bytes, as Obsidian writes them.
    pub fn to_json(&self) -> String {
        self.canvas.to_json()
    }
}

#[cfg(test)]
mod tests;
