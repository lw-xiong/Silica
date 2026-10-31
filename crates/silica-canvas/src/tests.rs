use super::*;

// ---- fixtures ---------------------------------------------------------------
// Copied from `crates/silica-formats/tests/canvas`: what the Obsidian probe
// wrote into a scratch vault (`*.in`) and what Obsidian 1.14.4 wrote back
// (`*.out`). The writer is silica-formats' job; what this crate owes is that
// a `Scene` with no edits, or with an edit undone, hands it the same model.

const ORDER_IN: &str = r##"{
  "nodes": [
    {"id": "t1", "type": "text", "text": "A", "x": 0, "y": 0, "width": 100, "height": 50},
    {"id": "g1", "type": "group", "x": -10, "y": -10, "width": 400, "height": 400, "label": "G1"},
    {"id": "t2", "type": "text", "text": "B", "x": 0, "y": 100, "width": 100, "height": 50, "color": "2"},
    {"id": "g2", "type": "group", "x": 500, "y": 500, "width": 100, "height": 100},
    {"id": "t3", "type": "text", "text": "C", "x": 0, "y": 200, "width": 100, "height": 50}
  ],
  "edges": [
    {"id": "e2", "fromNode": "t2", "fromSide": "bottom", "toNode": "t3", "toSide": "top"},
    {"id": "e1", "fromNode": "t1", "fromSide": "bottom", "toNode": "t2", "toSide": "top"},
    {"label": "all four", "color": "5", "toEnd": "none", "fromEnd": "arrow", "id": "e3", "fromNode": "t3", "fromSide": "right", "toNode": "t1", "toSide": "right"}
  ]
}
"##;

const ORDER_OUT: &str = concat!(
    "{\n",
    "\t\"nodes\":[\n",
    "\t\t{\"id\":\"g1\",\"type\":\"group\",\"x\":-10,\"y\":-10,\"width\":400,\"height\":400,\"label\":\"G1\"},\n",
    "\t\t{\"id\":\"g2\",\"type\":\"group\",\"x\":500,\"y\":500,\"width\":100,\"height\":100},\n",
    "\t\t{\"id\":\"t1\",\"type\":\"text\",\"text\":\"A\",\"x\":0,\"y\":0,\"width\":100,\"height\":50},\n",
    "\t\t{\"id\":\"t2\",\"type\":\"text\",\"text\":\"B\",\"x\":0,\"y\":100,\"width\":100,\"height\":50,\"color\":\"2\"},\n",
    "\t\t{\"id\":\"t3\",\"type\":\"text\",\"text\":\"C\",\"x\":10,\"y\":210,\"width\":100,\"height\":50}\n",
    "\t],\n",
    "\t\"edges\":[\n",
    "\t\t{\"id\":\"e2\",\"fromNode\":\"t2\",\"fromSide\":\"bottom\",\"toNode\":\"t3\",\"toSide\":\"top\"},\n",
    "\t\t{\"id\":\"e1\",\"fromNode\":\"t1\",\"fromSide\":\"bottom\",\"toNode\":\"t2\",\"toSide\":\"top\"},\n",
    "\t\t{\"id\":\"e3\",\"fromNode\":\"t3\",\"fromSide\":\"right\",\"toNode\":\"t1\",\"toSide\":\"right\",\"fromEnd\":\"arrow\",\"toEnd\":\"none\",\"color\":\"5\",\"label\":\"all four\"}\n",
    "\t]\n",
    "}",
);

const SCRAMBLED_OUT: &str = concat!(
    "{\n",
    "\t\"pluginTop\":{\"k\":1},\n",
    "\t\"edges\":[\n",
    "\t\t{\"id\":\"e1\",\"pluginEdge\":true,\"fromNode\":\"t1\",\"fromSide\":\"right\",\"toNode\":\"f1\",\"toSide\":\"left\",\"color\":\"2\",\"label\":\"leads to\"},\n",
    "\t\t{\"id\":\"e2\",\"fromNode\":\"l1\",\"fromSide\":\"top\",\"toNode\":\"f1\",\"toSide\":\"bottom\",\"fromEnd\":\"arrow\",\"toEnd\":\"none\"},\n",
    "\t\t{\"id\":\"e3\",\"fromNode\":\"f1\",\"fromSide\":\"bottom\",\"toNode\":\"t1\",\"toSide\":\"top\",\"color\":\"#00ff00\"},\n",
    "\t\t{\"id\":\"7ef5a6ac5cffca99\",\"fromNode\":\"t1\",\"fromSide\":\"top\",\"toNode\":\"f1\",\"toSide\":\"left\"}\n",
    "\t],\n",
    "\t\"nodes\":[\n",
    "\t\t{\"backgroundStyle\":\"cover\",\"type\":\"group\",\"id\":\"g1\",\"x\":-20,\"y\":-20,\"width\":900,\"height\":800,\"color\":\"4\",\"label\":\"Area\",\"background\":\"pic.png\"},\n",
    "\t\t{\"id\":\"g2\",\"type\":\"group\",\"x\":1000,\"y\":1000,\"width\":200,\"height\":100},\n",
    "\t\t{\n",
    "\t\t\t\"text\":\"Hello\",\n",
    "\t\t\t\"type\":\"text\",\n",
    "\t\t\t\"id\":\"t1\",\n",
    "\t\t\t\"pluginNode\":{\"k\":2},\n",
    "\t\t\t\"x\":0,\n",
    "\t\t\t\"y\":0,\n",
    "\t\t\t\"width\":250,\n",
    "\t\t\t\"height\":60,\n",
    "\t\t\t\"color\":\"1\"\n",
    "\t\t},\n",
    "\t\t{\"subpath\":\"#Goals\",\"file\":\"Plan.md\",\"type\":\"file\",\"id\":\"f1\",\"x\":300,\"y\":0,\"width\":400,\"height\":400,\"color\":\"#ff0000\"},\n",
    "\t\t{\"url\":\"https://example.com\",\"type\":\"link\",\"id\":\"l1\",\"x\":0,\"y\":200,\"width\":300,\"height\":200}\n",
    "\t]\n",
    "}",
);

const NOEDGES_OUT: &str = concat!(
    "{\n",
    "\t\"nodes\":[\n",
    "\t\t{\"id\":\"only\",\"type\":\"text\",\"text\":\"Alone\",\"x\":40,\"y\":30,\"width\":250,\"height\":60}\n",
    "\t],\n",
    "\t\"edges\":[\n",
    "\t\t{\"id\":\"e37fb57204c4107d\",\"fromNode\":\"only\",\"fromSide\":\"top\",\"toNode\":\"f772428526c489f6\",\"toSide\":\"bottom\"}\n",
    "\t]\n",
    "}",
);

// ---- helpers -----------------------------------------------------------------

fn card(id: &str, x: f64, y: f64, w: f64, h: f64) -> Node {
    let mut n = Node::new(id, NodeType::Text, x, y, w, h);
    n.text = Some(id.to_string());
    n
}

fn group(id: &str, x: f64, y: f64, w: f64, h: f64) -> Node {
    Node::new(id, NodeType::Group, x, y, w, h)
}

fn canvas(nodes: Vec<Node>, edges: Vec<Edge>) -> Canvas {
    let mut c = Canvas::default();
    c.nodes = nodes;
    c.edges = edges;
    c
}

fn sided(id: &str, from: &str, from_side: Side, to: &str, to_side: Side) -> Edge {
    let mut e = Edge::new(id, from, to);
    e.from_side = Some(from_side);
    e.to_side = Some(to_side);
    e
}

fn ids(canvas: &Canvas) -> Vec<&str> {
    canvas.nodes.iter().map(|n| n.id.as_str()).collect()
}

fn rect_of(scene: &Scene, id: &str) -> Rect {
    Rect::of(scene.node(id).expect("the node exists"))
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn close_point(a: Point, b: Point) -> bool {
    close(a.x, b.x) && close(a.y, b.y)
}

/// A pair of cards side by side with a straight edge between them, the
/// smallest canvas most tests want.
fn two_cards() -> Scene {
    Scene::new(canvas(
        vec![
            card("a", 0.0, 0.0, 100.0, 100.0),
            card("b", 300.0, 0.0, 100.0, 100.0),
        ],
        vec![sided("ab", "a", Side::Right, "b", Side::Left)],
    ))
}

// ---- Rect ----------------------------------------------------------------------

#[test]
fn between_takes_the_corners_either_way_round() {
    let r = Rect::between(Point::new(10.0, 50.0), Point::new(-20.0, 5.0));
    assert_eq!(r, Rect::new(-20.0, 5.0, 30.0, 45.0));
    assert_eq!(
        Rect::between(Point::new(-20.0, 5.0), Point::new(10.0, 50.0)),
        r
    );
    assert_eq!(
        Rect::between(Point::new(1.0, 1.0), Point::new(1.0, 1.0)),
        Rect::new(1.0, 1.0, 0.0, 0.0)
    );
}

#[test]
fn from_center_puts_the_centre_where_asked() {
    let r = Rect::from_center(Point::new(50.0, -10.0), 20.0, 40.0);
    assert_eq!(r, Rect::new(40.0, -30.0, 20.0, 40.0));
    assert_eq!(r.center(), Point::new(50.0, -10.0));
}

#[test]
fn union_covers_both_and_intersection_is_the_overlap() {
    let a = Rect::new(0.0, 0.0, 100.0, 100.0);
    let b = Rect::new(50.0, -50.0, 100.0, 100.0);
    assert_eq!(a.union(&b), Rect::new(0.0, -50.0, 150.0, 150.0));
    assert_eq!(b.union(&a), a.union(&b));
    assert_eq!(a.intersection(&b), Some(Rect::new(50.0, 0.0, 50.0, 50.0)));
    assert_eq!(b.intersection(&a), a.intersection(&b));
    // Touching along an edge is no overlap, nor is being apart.
    assert_eq!(a.intersection(&Rect::new(100.0, 0.0, 10.0, 10.0)), None);
    assert_eq!(a.intersection(&Rect::new(500.0, 500.0, 10.0, 10.0)), None);
    // One inside the other: the inner one.
    let inner = Rect::new(10.0, 10.0, 20.0, 20.0);
    assert_eq!(a.intersection(&inner), Some(inner));
}

#[test]
fn encloses_needs_the_whole_rectangle_inside_and_intersects_an_overlap() {
    let a = Rect::new(0.0, 0.0, 100.0, 100.0);
    assert!(a.encloses(&Rect::new(10.0, 10.0, 20.0, 20.0)));
    assert!(a.encloses(&a), "a rectangle encloses itself");
    assert!(!a.encloses(&Rect::new(90.0, 10.0, 20.0, 20.0)));
    assert!(a.intersects(&Rect::new(90.0, 10.0, 20.0, 20.0)));
    assert!(
        !a.intersects(&Rect::new(100.0, 0.0, 20.0, 20.0)),
        "touching is not overlapping"
    );
    assert!(!a.intersects(&Rect::new(-50.0, -50.0, 20.0, 20.0)));
    assert!(a.contains(Point::new(100.0, 100.0)), "the border counts");
    assert!(!a.contains(Point::new(100.1, 50.0)));
    assert_eq!(a.translate(5.0, -5.0), Rect::new(5.0, -5.0, 100.0, 100.0));
    assert_eq!(a.inflate(10.0), Rect::new(-10.0, -10.0, 120.0, 120.0));
    assert_eq!(a.right(), 100.0);
    assert_eq!(a.bottom(), 100.0);
}

#[test]
fn bounds_spans_the_nodes_and_is_none_for_none() {
    let nodes = [
        card("a", 0.0, 0.0, 100.0, 50.0),
        card("b", 400.0, -100.0, 100.0, 50.0),
    ];
    assert_eq!(bounds(&nodes), Some(Rect::new(0.0, -100.0, 500.0, 150.0)));
    assert_eq!(bounds(&[]), None);
}

// ---- Viewport --------------------------------------------------------------------

#[test]
fn screen_and_canvas_coordinates_invert_each_other() {
    let size = (800.0, 600.0);
    for zoom in [-2.5, -1.0, 0.0, 0.5] {
        let vp = Viewport {
            x: 100.0,
            y: -50.0,
            zoom,
        };
        let p = Point::new(123.0, 456.0);
        assert!(close_point(vp.to_canvas(vp.to_screen(p, size), size), p));
        let s = Point::new(10.0, 590.0);
        assert!(close_point(vp.to_screen(vp.to_canvas(s, size), size), s));
    }
}

#[test]
fn the_viewport_position_is_the_canvas_point_at_the_centre_of_the_view() {
    let vp = Viewport {
        x: 100.0,
        y: -50.0,
        zoom: 1.0,
    };
    let size = (800.0, 600.0);
    assert_eq!(vp.scale(), 2.0);
    assert_eq!(
        vp.to_screen(Point::new(100.0, -50.0), size),
        Point::new(400.0, 300.0)
    );
    // One canvas unit is `scale` pixels.
    assert_eq!(
        vp.to_screen(Point::new(101.0, -50.0), size),
        Point::new(402.0, 300.0)
    );
    assert_eq!(
        vp.rect_to_screen(Rect::new(100.0, -50.0, 10.0, 20.0), size),
        Rect::new(400.0, 300.0, 20.0, 40.0)
    );
}

#[test]
fn panning_moves_the_content_with_the_pointer() {
    let size = (800.0, 600.0);
    let mut vp = Viewport {
        x: 0.0,
        y: 0.0,
        zoom: -1.0,
    };
    let under = vp.to_canvas(Point::new(100.0, 100.0), size);
    vp.pan(10.0, -30.0);
    assert!(close_point(
        vp.to_screen(under, size),
        Point::new(110.0, 70.0)
    ));
}

#[test]
fn zoom_at_keeps_the_point_under_the_pointer_and_clamps() {
    let size = (800.0, 600.0);
    let at = Point::new(100.0, 500.0);
    let mut vp = Viewport::default();
    let before = vp.to_canvas(at, size);
    vp.zoom_at(0.5, at, size);
    assert!(close(vp.zoom, 0.5f64.clamp(ZOOM_MIN, ZOOM_MAX)));
    assert!(close_point(vp.to_canvas(at, size), before));
    vp.zoom_at(1000.0, at, size);
    assert_eq!(vp.zoom, ZOOM_MAX);
    assert!(close_point(vp.to_canvas(at, size), before));
    vp.zoom_at(-1000.0, at, size);
    assert_eq!(vp.zoom, ZOOM_MIN);
    assert!(close_point(vp.to_canvas(at, size), before));
}

#[test]
fn fit_frames_the_rectangle_with_padding_and_clamps() {
    let size = (800.0, 600.0);
    let rect = Rect::new(0.0, 0.0, 1400.0, 500.0);
    let vp = Viewport::fit(rect, size, 50.0);
    // Width is the tighter side: 700 px for 1400 units.
    let raw = 0.5f64.log2();
    assert!(close(vp.zoom, raw.clamp(ZOOM_MIN, ZOOM_MAX)));
    assert_eq!(Point::new(vp.x, vp.y), rect.center());
    if close(vp.zoom, raw) {
        let on_screen = vp.rect_to_screen(rect, size);
        assert!(close(on_screen.x, 50.0) && close(on_screen.width, 700.0));
        assert!(close(on_screen.center().y, 300.0));
    }
    // A tiny rectangle would zoom in without limit; the range stops it.
    let tiny = Viewport::fit(Rect::new(0.0, 0.0, 1.0, 1.0), size, 0.0);
    assert_eq!(tiny.zoom, ZOOM_MAX);
    // A huge one zooms out no further than the range either.
    let huge = Viewport::fit(Rect::new(0.0, 0.0, 1e9, 1e9), size, 0.0);
    assert_eq!(huge.zoom, ZOOM_MIN);
}

#[test]
fn fit_nodes_frames_their_bounds_and_is_none_for_none() {
    let size = (800.0, 600.0);
    let nodes = [
        card("a", 0.0, 0.0, 100.0, 100.0),
        card("b", 400.0, 0.0, 100.0, 100.0),
    ];
    let vp = Viewport::fit_nodes(&nodes, size, 40.0).expect("two nodes frame");
    assert_eq!(
        vp,
        Viewport::fit(Rect::new(0.0, 0.0, 500.0, 100.0), size, 40.0)
    );
    assert_eq!(Viewport::fit_nodes(&[], size, 40.0), None);
}

#[test]
fn visible_is_the_canvas_rectangle_the_view_shows() {
    let size = (800.0, 600.0);
    assert_eq!(
        Viewport::default().visible(size),
        Rect::new(-400.0, -300.0, 800.0, 600.0)
    );
    let vp = Viewport {
        x: 100.0,
        y: 100.0,
        zoom: 1.0,
    };
    let (w, h) = (size.0 / vp.scale(), size.1 / vp.scale());
    assert_eq!(
        vp.visible(size),
        Rect::new(100.0 - w / 2.0, 100.0 - h / 2.0, w, h)
    );
}

#[test]
fn zoom_step_zooms_around_the_centre_and_reset_goes_home() {
    let size = (800.0, 600.0);
    let mut vp = Viewport {
        x: 100.0,
        y: 50.0,
        zoom: 0.0,
    };
    vp.zoom_step(0.5, size);
    assert!(close(vp.zoom, 0.5f64.clamp(ZOOM_MIN, ZOOM_MAX)));
    assert!(
        close(vp.x, 100.0) && close(vp.y, 50.0),
        "the centre stays put"
    );
    vp.zoom_step(-100.0, size);
    assert_eq!(vp.zoom, ZOOM_MIN);
    vp.reset();
    assert_eq!(vp, Viewport::default());
}

// ---- Color ---------------------------------------------------------------------

#[test]
fn colors_parse_presets_hex_and_reject_garbage() {
    for n in 1..=6u8 {
        assert_eq!(Color::parse(Some(&n.to_string())), Color::Preset(n));
    }
    assert_eq!(Color::parse(Some("#ff0000")), Color::Hex(0xff0000));
    assert_eq!(
        Color::parse(Some("#FF00aB")),
        Color::Hex(0xff00ab),
        "case does not matter"
    );
    for bad in [
        "0", "7", "9", "", " 1", "red", "#abc", "#gggggg", "#ff00000", "ff0000",
    ] {
        assert_eq!(Color::parse(Some(bad)), Color::Default, "{bad:?}");
    }
    assert_eq!(Color::parse(None), Color::Default);
}

#[test]
fn colors_write_what_the_file_carries_and_round_trip() {
    assert_eq!(Color::Default.to_file(), None);
    assert_eq!(Color::Preset(3).to_file().as_deref(), Some("3"));
    assert_eq!(Color::Hex(0x00ff00).to_file().as_deref(), Some("#00ff00"));
    assert_eq!(
        Color::Hex(0xab).to_file().as_deref(),
        Some("#0000ab"),
        "six digits, zero padded"
    );
    for c in [Color::Default, Color::Preset(6), Color::Hex(0x123abc)] {
        assert_eq!(Color::parse(c.to_file().as_deref()), c);
    }
}

// ---- anchors and sides -----------------------------------------------------------

#[test]
fn anchors_are_the_middles_of_the_sides() {
    let r = Rect::new(10.0, 20.0, 100.0, 40.0);
    assert_eq!(anchor(&r, Side::Top), Point::new(60.0, 20.0));
    assert_eq!(anchor(&r, Side::Right), Point::new(110.0, 40.0));
    assert_eq!(anchor(&r, Side::Bottom), Point::new(60.0, 60.0));
    assert_eq!(anchor(&r, Side::Left), Point::new(10.0, 40.0));
    for side in [Side::Top, Side::Right, Side::Bottom, Side::Left] {
        let n = side_normal(side);
        assert!(close(n.x * n.x + n.y * n.y, 1.0));
        // Stepping along the normal from the anchor leaves the rectangle.
        let a = anchor(&r, side);
        assert!(!r.contains(Point::new(a.x + n.x, a.y + n.y)));
    }
}

#[test]
fn facing_side_is_the_side_the_target_lies_beyond() {
    let square = Rect::new(0.0, 0.0, 100.0, 100.0);
    assert_eq!(facing_side(&square, Point::new(300.0, 50.0)), Side::Right);
    assert_eq!(facing_side(&square, Point::new(-300.0, 50.0)), Side::Left);
    assert_eq!(facing_side(&square, Point::new(50.0, 300.0)), Side::Bottom);
    assert_eq!(facing_side(&square, Point::new(50.0, -300.0)), Side::Top);
    // A point in a quadrant still picks by which side's cone it is in.
    assert_eq!(facing_side(&square, Point::new(300.0, 120.0)), Side::Right);
    assert_eq!(facing_side(&square, Point::new(-10.0, -300.0)), Side::Top);
    // A wide card is reached from above or below for the same diagonal
    // point that a tall card is reached from the side.
    let wide = Rect::new(0.0, 0.0, 400.0, 100.0);
    let tall = Rect::new(0.0, 0.0, 100.0, 400.0);
    assert_eq!(facing_side(&wide, Point::new(300.0, 300.0)), Side::Bottom);
    assert_eq!(facing_side(&tall, Point::new(300.0, 300.0)), Side::Right);
    // Inside the rectangle is still an answer.
    assert_eq!(facing_side(&square, Point::new(60.0, 50.0)), Side::Right);
}

// ---- edge geometry ---------------------------------------------------------------

#[test]
fn a_straight_edge_runs_between_the_anchors() {
    let a = Rect::new(0.0, 0.0, 100.0, 100.0);
    let b = Rect::new(300.0, 0.0, 100.0, 100.0);
    let path = edge_path(&a, Side::Right, &b, Side::Left);
    // The curve starts and ends `EDGE_INSET` out from the cards' edges; the
    // stubs (or arrows) cover the rest.
    assert_eq!(path.from, Point::new(100.0 + EDGE_INSET, 50.0));
    assert_eq!(path.to, Point::new(300.0 - EDGE_INSET, 50.0));
    assert_eq!(
        edge_stub(anchor(&a, Side::Right), Side::Right),
        (Point::new(100.0, 50.0), path.from)
    );
    assert!(
        path.c1.x > path.from.x && path.c1.y == 50.0,
        "leaves along the right normal"
    );
    assert!(
        path.c2.x < path.to.x && path.c2.y == 50.0,
        "arrives along the left normal"
    );
    assert_eq!(path.point_at(0.0), path.from);
    assert_eq!(path.point_at(1.0), path.to);
    assert!(close_point(path.midpoint(), Point::new(200.0, 50.0)));
    assert!(close_point(path.midpoint(), path.point_at(0.5)));
    let pts = path.flatten(4);
    assert_eq!(pts.len(), 5);
    assert_eq!((pts[0], pts[4]), (path.from, path.to));
    assert!(
        pts.windows(2).all(|w| w[0].x <= w[1].x),
        "monotone along a straight edge"
    );
    assert!(close(path.length(), 200.0 - 2.0 * EDGE_INSET));
    assert!(close(path.distance_to(Point::new(200.0, 150.0)), 100.0));
    assert!(
        close(
            path.distance_to(Point::new(400.0, 50.0)),
            100.0 + EDGE_INSET
        ),
        "beyond the end, the end is nearest"
    );
    assert!(path.distance_to(path.midpoint()) < 1e-9);
    assert_eq!(
        path.flatten(0).len(),
        2,
        "no segments still gives both ends"
    );
}

#[test]
fn tangents_follow_the_side_normals_at_the_ends() {
    let a = Rect::new(0.0, 0.0, 100.0, 100.0);
    let below_right = Rect::new(300.0, 300.0, 100.0, 100.0);
    let path = edge_path(&a, Side::Right, &below_right, Side::Left);
    // Leaving a right side heads right; arriving at a left side heads right too.
    assert!(close_point(path.tangent_at(0.0), Point::new(1.0, 0.0)));
    assert!(close_point(path.tangent_at(1.0), Point::new(1.0, 0.0)));
    let down = edge_path(&a, Side::Bottom, &below_right, Side::Top);
    assert!(close_point(down.tangent_at(0.0), Point::new(0.0, 1.0)));
    assert!(close_point(down.tangent_at(1.0), Point::new(0.0, 1.0)));
    // In between it is a unit vector pointing roughly from a towards b.
    let mid = path.tangent_at(0.5);
    assert!(close(mid.x * mid.x + mid.y * mid.y, 1.0));
    assert!(mid.x > 0.0 && mid.y > 0.0);
    // A free end: the arrow head's base sits behind the tip along the tangent.
    let head = arrow_head(path.to, path.tangent_at(1.0), ARROW);
    assert!(head[1].x < path.to.x && head[2].x < path.to.x);
}

#[test]
fn a_degenerate_curve_still_has_a_tangent() {
    let flat = EdgePath {
        from: Point::new(0.0, 0.0),
        c1: Point::new(0.0, 0.0),
        c2: Point::new(10.0, 5.0),
        to: Point::new(10.0, 5.0),
    };
    // Control points on the anchors: zero derivative at both ends, so the chord.
    let chord = Point::new(10.0 / 125f64.sqrt(), 5.0 / 125f64.sqrt());
    assert!(close_point(flat.tangent_at(0.0), chord));
    assert!(close_point(flat.tangent_at(1.0), chord));
    let dot = EdgePath {
        from: Point::new(3.0, 3.0),
        c1: Point::new(3.0, 3.0),
        c2: Point::new(3.0, 3.0),
        to: Point::new(3.0, 3.0),
    };
    assert_eq!(dot.tangent_at(0.5), Point::new(1.0, 0.0));
    assert_eq!(dot.length(), 0.0);
    assert_eq!(dot.bounds(), Rect::new(3.0, 3.0, 0.0, 0.0));
}

#[test]
fn edge_bounds_contain_every_point_of_the_curve() {
    let a = Rect::new(0.0, 0.0, 100.0, 100.0);
    let b = Rect::new(-300.0, 400.0, 100.0, 100.0);
    for (fs, ts) in [
        (Side::Right, Side::Left),
        (Side::Bottom, Side::Top),
        (Side::Top, Side::Bottom),
        (Side::Left, Side::Right),
    ] {
        let path = edge_path(&a, fs, &b, ts);
        let bounds = path.bounds();
        for p in path.flatten(64) {
            assert!(
                bounds.inflate(1e-9).contains(p),
                "{fs:?}->{ts:?} {p:?} outside {bounds:?}"
            );
        }
        assert!(bounds.contains(path.from) && bounds.contains(path.to));
        assert!(
            path.length() >= path.from.distance(path.to) - 1e-9,
            "never shorter than the chord"
        );
    }
}

#[test]
fn edge_path_points_is_the_curve_behind_edge_path() {
    let a = Rect::new(0.0, 0.0, 100.0, 100.0);
    let b = Rect::new(300.0, 0.0, 100.0, 100.0);
    assert_eq!(
        edge_path(&a, Side::Right, &b, Side::Left),
        edge_path_points(
            anchor(&a, Side::Right),
            Side::Right,
            anchor(&b, Side::Left),
            Side::Left
        )
    );
    // Two anchors on top of each other still get a drawable curve.
    let p = Point::new(5.0, 5.0);
    let tiny = edge_path_points(p, Side::Right, p, Side::Left);
    assert!(
        tiny.c1.x > p.x && tiny.c2.x < p.x,
        "the minimum reach keeps the loop open"
    );
}

#[test]
fn arrow_heads_point_along_the_direction() {
    let head = arrow_head(Point::new(100.0, 0.0), Point::new(1.0, 0.0), ARROW);
    assert_eq!(head[0], Point::new(100.0, 0.0), "the tip comes first");
    let base = 100.0 - ARROW;
    assert!(close_point(head[1], Point::new(base, ARROW_WIDTH / 2.0)));
    assert!(close_point(head[2], Point::new(base, -ARROW_WIDTH / 2.0)));
    let down = arrow_head(Point::new(0.0, 0.0), Point::new(0.0, 1.0), ARROW);
    assert!(close_point(down[1], Point::new(-ARROW_WIDTH / 2.0, -ARROW)));
    assert!(close_point(down[2], Point::new(ARROW_WIDTH / 2.0, -ARROW)));
    // The head at `from` points against the start tangent: its base lies
    // further along the curve than the tip.
    let path = edge_path(
        &Rect::new(0.0, 0.0, 100.0, 100.0),
        Side::Right,
        &Rect::new(300.0, 0.0, 100.0, 100.0),
        Side::Left,
    );
    let t = path.tangent_at(0.0);
    let back = arrow_head(path.from, Point::new(-t.x, -t.y), ARROW);
    assert!(back[1].x > path.from.x && back[2].x > path.from.x);
}

#[test]
fn edge_ends_apply_obsidians_defaults() {
    let mut e = Edge::new("e", "a", "b");
    assert_eq!(edge_ends(&e), (false, true), "an arrow at the end only");
    e.from_end = Some(EndStyle::Arrow);
    assert_eq!(edge_ends(&e), (true, true));
    e.to_end = Some(EndStyle::None);
    assert_eq!(edge_ends(&e), (true, false));
    e.from_end = Some(EndStyle::None);
    e.to_end = Some(EndStyle::Arrow);
    assert_eq!(
        edge_ends(&e),
        (false, true),
        "the defaults written out say the same"
    );
}

// ---- hit testing -----------------------------------------------------------------

#[test]
fn hit_node_prefers_the_later_card_and_cards_over_groups() {
    let c = canvas(
        vec![
            group("g", 0.0, 0.0, 500.0, 500.0),
            card("a", 100.0, 100.0, 100.0, 100.0),
            card("b", 150.0, 150.0, 100.0, 100.0),
            group("over", 0.0, 0.0, 500.0, 500.0),
        ],
        vec![],
    );
    assert_eq!(
        hit_node(&c, Point::new(175.0, 175.0)),
        Some(2),
        "later card wins the overlap"
    );
    assert_eq!(
        hit_node(&c, Point::new(110.0, 110.0)),
        Some(1),
        "a group above a card still loses"
    );
    assert_eq!(
        hit_node(&c, Point::new(400.0, 400.0)),
        Some(3),
        "the later group's body"
    );
    assert_eq!(hit_node(&c, Point::new(600.0, 600.0)), None);
    assert_eq!(hit_node(&Canvas::default(), Point::new(0.0, 0.0)), None);
}

#[test]
fn hit_edge_takes_the_nearest_within_tolerance() {
    let c = canvas(
        vec![
            card("a", 0.0, 0.0, 100.0, 100.0),
            card("b", 300.0, 0.0, 100.0, 100.0),
            card("c", 0.0, 100.0, 100.0, 100.0),
            card("d", 300.0, 100.0, 100.0, 100.0),
        ],
        vec![
            sided("ab", "a", Side::Right, "b", Side::Left), // y = 50
            sided("cd", "c", Side::Right, "d", Side::Left), // y = 150
            Edge::new("dangling", "a", "gone"),
        ],
    );
    assert_eq!(hit_edge(&c, Point::new(200.0, 55.0), 8.0), Some(0));
    assert_eq!(
        hit_edge(&c, Point::new(200.0, 70.0), 8.0),
        None,
        "20 away misses"
    );
    assert_eq!(hit_edge(&c, Point::new(200.0, 70.0), 25.0), Some(0));
    assert_eq!(
        hit_edge(&c, Point::new(200.0, 90.0), 100.0),
        Some(0),
        "nearer to ab"
    );
    assert_eq!(
        hit_edge(&c, Point::new(200.0, 110.0), 100.0),
        Some(1),
        "nearer to cd"
    );
    assert_eq!(
        hit_edge(&c, Point::new(50.0, 50.0), 0.0),
        None,
        "inside a card is not on the line"
    );
}

#[test]
fn resolved_edge_path_fills_in_facing_sides_and_skips_a_missing_node() {
    let c = canvas(
        vec![
            card("a", 0.0, 0.0, 100.0, 100.0),
            card("b", 300.0, 0.0, 100.0, 100.0),
        ],
        vec![],
    );
    let plain = Edge::new("e", "a", "b");
    let path = resolved_edge_path(&c, &plain).expect("both nodes exist");
    assert_eq!(path.from, Point::new(100.0 + EDGE_INSET, 50.0));
    assert_eq!(path.to, Point::new(300.0 - EDGE_INSET, 50.0));
    let mut explicit = Edge::new("f", "a", "b");
    explicit.from_side = Some(Side::Top);
    explicit.to_side = Some(Side::Bottom);
    let path = resolved_edge_path(&c, &explicit).expect("both nodes exist");
    assert_eq!(path.from, Point::new(50.0, -EDGE_INSET));
    assert_eq!(path.to, Point::new(350.0, 100.0 + EDGE_INSET));
    assert!(resolved_edge_path(&c, &Edge::new("g", "a", "zzz")).is_none());
    assert!(resolved_edge_path(&c, &Edge::new("h", "zzz", "b")).is_none());
}

#[test]
fn a_marquee_and_the_view_take_the_nodes_they_touch() {
    let c = canvas(
        vec![
            card("a", 0.0, 0.0, 100.0, 100.0),
            card("b", 200.0, 0.0, 100.0, 100.0),
            card("c", 1000.0, 1000.0, 100.0, 100.0),
        ],
        vec![],
    );
    let marquee = Rect::new(50.0, 50.0, 200.0, 20.0);
    assert_eq!(
        nodes_in(&c, &marquee),
        vec![0, 1],
        "half covered still counts"
    );
    assert_eq!(visible_nodes(&c, &marquee), vec![0, 1]);
    assert_eq!(
        nodes_in(&c, &Rect::new(100.0, 0.0, 100.0, 100.0)),
        Vec::<usize>::new(),
        "touching edges is not inside"
    );
    assert_eq!(
        visible_nodes(&c, &Rect::new(-1e6, -1e6, 2e6, 2e6)),
        vec![0, 1, 2]
    );
}

#[test]
fn group_members_are_the_nodes_wholly_inside() {
    let c = canvas(
        vec![
            group("g", 0.0, 0.0, 500.0, 500.0),
            card("inside", 10.0, 10.0, 100.0, 100.0),
            card("straddling", 450.0, 10.0, 100.0, 100.0),
            group("nested", 200.0, 200.0, 100.0, 100.0),
            card("outside", 900.0, 900.0, 10.0, 10.0),
            card("flush", 400.0, 400.0, 100.0, 100.0),
        ],
        vec![],
    );
    assert_eq!(
        group_members(&c, 0),
        vec![1, 3, 5],
        "the group itself is not a member"
    );
    assert_eq!(group_members(&c, 3), Vec::<usize>::new());
    assert_eq!(group_members(&c, 99), Vec::<usize>::new());
}

// ---- resize handles ----------------------------------------------------------------

#[test]
fn handle_positions_sit_on_the_rectangle() {
    let r = Rect::new(0.0, 0.0, 200.0, 100.0);
    assert_eq!(Handle::Top.position(&r), Point::new(100.0, 0.0));
    assert_eq!(Handle::Right.position(&r), Point::new(200.0, 50.0));
    assert_eq!(Handle::Bottom.position(&r), Point::new(100.0, 100.0));
    assert_eq!(Handle::Left.position(&r), Point::new(0.0, 50.0));
    assert_eq!(Handle::TopLeft.position(&r), Point::new(0.0, 0.0));
    assert_eq!(Handle::TopRight.position(&r), Point::new(200.0, 0.0));
    assert_eq!(Handle::BottomLeft.position(&r), Point::new(0.0, 100.0));
    assert_eq!(Handle::BottomRight.position(&r), Point::new(200.0, 100.0));
    assert_eq!(Handle::ALL.len(), 8);
}

#[test]
fn each_handle_moves_only_its_sides() {
    let r = Rect::new(100.0, 100.0, 400.0, 300.0);
    let (dx, dy) = (10.0, 20.0);
    let cases = [
        (Handle::Top, Rect::new(100.0, 120.0, 400.0, 280.0)),
        (Handle::Right, Rect::new(100.0, 100.0, 410.0, 300.0)),
        (Handle::Bottom, Rect::new(100.0, 100.0, 400.0, 320.0)),
        (Handle::Left, Rect::new(110.0, 100.0, 390.0, 300.0)),
        (Handle::TopLeft, Rect::new(110.0, 120.0, 390.0, 280.0)),
        (Handle::TopRight, Rect::new(100.0, 120.0, 410.0, 280.0)),
        (Handle::BottomLeft, Rect::new(110.0, 100.0, 390.0, 320.0)),
        (Handle::BottomRight, Rect::new(100.0, 100.0, 410.0, 320.0)),
    ];
    for (handle, expected) in cases {
        assert_eq!(handle.resize(&r, dx, dy, MIN_NODE), expected, "{handle:?}");
        // The side opposite a moved one never moves.
        let out = handle.resize(&r, dx, dy, MIN_NODE);
        match handle {
            Handle::Left | Handle::TopLeft | Handle::BottomLeft => {
                assert_eq!(out.right(), r.right())
            }
            Handle::Right | Handle::TopRight | Handle::BottomRight => assert_eq!(out.x, r.x),
            _ => {}
        }
    }
}

#[test]
fn resizing_past_the_opposite_side_stops_at_the_minimum() {
    let r = Rect::new(100.0, 100.0, 400.0, 300.0);
    let (mw, mh) = MIN_NODE;
    let left = Handle::Left.resize(&r, 1000.0, 0.0, MIN_NODE);
    assert_eq!(
        left,
        Rect::new(r.right() - mw, 100.0, mw, 300.0),
        "the right side stays put"
    );
    let top = Handle::Top.resize(&r, 0.0, 1000.0, MIN_NODE);
    assert_eq!(top, Rect::new(100.0, r.bottom() - mh, 400.0, mh));
    let right = Handle::Right.resize(&r, -1000.0, 0.0, MIN_NODE);
    assert_eq!(right, Rect::new(100.0, 100.0, mw, 300.0));
    let bottom = Handle::Bottom.resize(&r, 0.0, -1000.0, MIN_NODE);
    assert_eq!(bottom, Rect::new(100.0, 100.0, 400.0, mh));
    let corner = Handle::TopLeft.resize(&r, 1000.0, 1000.0, MIN_NODE);
    assert_eq!(corner, Rect::new(r.right() - mw, r.bottom() - mh, mw, mh));
    let other = Handle::BottomRight.resize(&r, -1000.0, -1000.0, MIN_NODE);
    assert_eq!(other, Rect::new(100.0, 100.0, mw, mh));
    // Exactly at the minimum is allowed.
    let exact = Handle::Right.resize(&r, mw - 400.0, 0.0, MIN_NODE);
    assert_eq!(exact.width, mw);
}

#[test]
fn hit_handle_finds_the_handle_within_its_size_and_nothing_elsewhere() {
    let r = Rect::new(0.0, 0.0, 200.0, 100.0);
    let half = HANDLE / 2.0;
    assert_eq!(
        hit_handle(&r, Point::new(200.0 + half, 50.0), HANDLE),
        Some(Handle::Right)
    );
    assert_eq!(
        hit_handle(&r, Point::new(200.0 + half + 0.1, 50.0), HANDLE),
        None
    );
    assert_eq!(
        hit_handle(&r, Point::new(0.0, 0.0), HANDLE),
        Some(Handle::TopLeft)
    );
    assert_eq!(
        hit_handle(&r, Point::new(100.0, 0.0), HANDLE),
        Some(Handle::Top)
    );
    assert_eq!(
        hit_handle(&r, Point::new(200.0, 100.0), HANDLE),
        Some(Handle::BottomRight)
    );
    assert_eq!(
        hit_handle(&r, Point::new(2.0, 97.0), HANDLE),
        Some(Handle::BottomLeft)
    );
    assert_eq!(
        hit_handle(&r, Point::new(100.0, 50.0), HANDLE),
        None,
        "the middle is a drag, not a resize"
    );
}

// ---- snapping and ids ----------------------------------------------------------------

#[test]
fn snap_rounds_to_the_grid() {
    assert_eq!(snap(0.0), 0.0);
    assert_eq!(snap(GRID * 1.4), GRID);
    assert_eq!(snap(GRID * 1.6), 2.0 * GRID);
    assert_eq!(snap(-GRID * 0.6), -GRID);
    assert_eq!(snap(GRID * 7.0), GRID * 7.0);
    assert_eq!(
        snap_point(Point::new(GRID * 2.3, -GRID * 2.7)),
        Point::new(2.0 * GRID, -3.0 * GRID)
    );
}

#[test]
fn new_ids_are_sixteen_lowercase_hex_digits_and_differ() {
    let a = new_id();
    let b = new_id();
    for id in [&a, &b] {
        assert_eq!(id.len(), 16, "{id}");
        assert!(
            id.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "{id}"
        );
    }
    assert_ne!(a, b);
}

// ---- Selection -------------------------------------------------------------------------

#[test]
fn selection_toggles_and_queries() {
    let mut s = Selection::default();
    assert!(s.is_empty());
    s.toggle_node("a");
    s.toggle_edge("e");
    assert!(s.has_node("a") && s.has_edge("e") && !s.is_empty());
    assert!(!s.has_node("e") && !s.has_edge("a"));
    s.toggle_node("a");
    assert!(!s.has_node("a"), "a second toggle removes");
    s.toggle_node("b");
    s.toggle_node("c");
    assert_eq!(s.nodes, ["b", "c"], "order of selection is kept");
    s.clear();
    assert!(s.is_empty());
    assert_eq!(
        Selection::only_node("n"),
        Selection {
            nodes: vec!["n".into()],
            edges: vec![]
        }
    );
    assert_eq!(
        Selection::only_edge("e"),
        Selection {
            nodes: vec![],
            edges: vec!["e".into()]
        }
    );
}

#[test]
fn retain_present_drops_ids_the_canvas_lost() {
    let scene = two_cards();
    let mut s = Selection {
        nodes: vec!["a".into(), "gone".into(), "b".into()],
        edges: vec!["ab".into(), "nope".into()],
    };
    s.retain_present(&scene.canvas);
    assert_eq!(s.nodes, ["a", "b"]);
    assert_eq!(s.edges, ["ab"]);
}

// ---- Scene: history ---------------------------------------------------------------------

#[test]
fn a_drag_is_one_undo_step() {
    let mut scene = two_cards();
    assert!(!scene.can_undo() && !scene.can_redo() && !scene.is_dirty());
    scene.selection = Selection::only_node("a");
    scene.begin();
    for _ in 0..10 {
        scene.move_selection(1.0, 2.0);
    }
    scene.commit();
    assert_eq!(rect_of(&scene, "a"), Rect::new(10.0, 20.0, 100.0, 100.0));
    assert!(scene.can_undo() && !scene.can_redo() && scene.is_dirty());
    assert!(scene.undo());
    assert_eq!(rect_of(&scene, "a"), Rect::new(0.0, 0.0, 100.0, 100.0));
    assert!(!scene.can_undo() && scene.can_redo());
    assert!(!scene.undo(), "nothing left");
    assert!(scene.redo());
    assert_eq!(rect_of(&scene, "a"), Rect::new(10.0, 20.0, 100.0, 100.0));
    assert!(!scene.redo());
}

#[test]
fn a_commit_without_change_records_nothing_and_begin_twice_keeps_the_first_snapshot() {
    let mut scene = two_cards();
    scene.begin();
    scene.commit();
    assert!(!scene.can_undo() && !scene.is_dirty());
    scene.selection = Selection::only_node("a");
    scene.begin();
    scene.move_selection(5.0, 0.0);
    scene.begin();
    scene.move_selection(5.0, 0.0);
    scene.commit();
    assert!(scene.undo());
    assert_eq!(
        rect_of(&scene, "a").x,
        0.0,
        "one step back is the very start"
    );
    // A mutator called with no `begin` still changes the canvas; it is
    // simply not undoable, which is what a view that forgot `begin` gets.
    scene.move_selection(1.0, 0.0);
    scene.commit();
    assert!(!scene.can_undo());
    assert_eq!(rect_of(&scene, "a").x, 1.0);
}

#[test]
fn edit_wraps_a_closure_and_a_new_edit_clears_the_redo_stack() {
    let mut scene = two_cards();
    scene.edit(|c| c.nodes[0].x = 50.0);
    assert!(scene.can_undo() && scene.is_dirty());
    scene.edit(|_| {});
    assert_eq!(
        scene.undo_depth(),
        1,
        "an edit that changes nothing is not a step"
    );
    scene.undo();
    assert!(scene.can_redo());
    scene.edit(|c| c.nodes[1].y = 7.0);
    assert!(!scene.can_redo(), "a new edit forgets what was undone");
    assert_eq!(rect_of(&scene, "a").x, 0.0);
    assert_eq!(rect_of(&scene, "b").y, 7.0);
}

#[test]
fn reload_takes_the_new_file_and_forgets_the_history() {
    let mut scene = two_cards();
    scene.edit(|c| c.nodes[0].x = 50.0);
    scene.undo();
    scene.selection = Selection {
        nodes: vec!["a".into(), "b".into()],
        edges: vec!["ab".into()],
    };
    scene.reload(canvas(vec![card("b", 0.0, 0.0, 10.0, 10.0)], vec![]));
    assert_eq!(ids(&scene.canvas), ["b"]);
    assert_eq!(scene.selection, Selection::only_node("b"));
    assert!(!scene.can_undo() && !scene.can_redo() && !scene.is_dirty());
}

#[test]
fn mark_saved_clears_dirty_and_history_moves_dirty_it_again() {
    let mut scene = two_cards();
    scene.edit(|c| c.nodes[0].x = 50.0);
    scene.mark_saved();
    assert!(!scene.is_dirty());
    scene.undo();
    assert!(
        scene.is_dirty(),
        "the file no longer matches what was saved"
    );
    scene.mark_saved();
    scene.redo();
    assert!(scene.is_dirty());
}

#[test]
fn undo_and_redo_prune_the_selection() {
    let mut scene = Scene::new(Canvas::default());
    scene.begin();
    let id = scene.add_text_card(Point::new(0.0, 0.0), "a");
    scene.commit();
    assert_eq!(scene.selection, Selection::only_node(&id));
    scene.undo();
    assert!(scene.selection.is_empty());
    scene.redo();
    assert!(
        scene.selection.is_empty(),
        "the node is back; the selection was already gone"
    );
}

// ---- Scene: lookups ----------------------------------------------------------------------

#[test]
fn lookups_find_nodes_and_edges_by_id() {
    let mut scene = two_cards();
    assert_eq!(scene.node_index("b"), Some(1));
    assert_eq!(scene.node_index("zzz"), None);
    assert_eq!(scene.node("a").map(|n| n.x), Some(0.0));
    assert!(scene.node("zzz").is_none());
    assert_eq!(scene.edge("ab").map(|e| e.from_node.as_str()), Some("a"));
    assert!(scene.edge("zzz").is_none());
    scene.node_mut("a").expect("a exists").x = 9.0;
    scene.edge_mut("ab").expect("ab exists").label = Some("L".into());
    assert_eq!(scene.node("a").map(|n| n.x), Some(9.0));
    assert_eq!(scene.edge("ab").and_then(|e| e.label.as_deref()), Some("L"));
    assert!(scene.node_mut("zzz").is_none() && scene.edge_mut("zzz").is_none());
}

#[test]
fn selection_bounds_spans_the_selected_nodes() {
    let mut scene = two_cards();
    assert_eq!(scene.selection_bounds(), None);
    scene.selection = Selection::only_node("a");
    assert_eq!(
        scene.selection_bounds(),
        Some(Rect::new(0.0, 0.0, 100.0, 100.0))
    );
    scene.selection.toggle_node("b");
    scene.selection.toggle_node("missing");
    assert_eq!(
        scene.selection_bounds(),
        Some(Rect::new(0.0, 0.0, 400.0, 100.0))
    );
    scene.selection = Selection::only_edge("ab");
    assert_eq!(
        scene.selection_bounds(),
        None,
        "edges have no bounds of their own"
    );
}

#[test]
fn edges_of_lists_both_directions_in_file_order() {
    let mut scene = two_cards();
    scene.canvas.nodes.push(card("c", 0.0, 300.0, 100.0, 100.0));
    scene.canvas.edges.push(Edge::new("ca", "c", "a"));
    scene.canvas.edges.push(Edge::new("bc", "b", "c"));
    let of = |id: &str| {
        scene
            .edges_of(id)
            .into_iter()
            .map(|e| e.id.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(of("a"), ["ab", "ca"]);
    assert_eq!(of("c"), ["ca", "bc"]);
    assert_eq!(of("zzz"), Vec::<&str>::new());
}

#[test]
fn jump_targets_are_the_labelled_groups_in_order() {
    let mut named = group("named", 0.0, 0.0, 10.0, 10.0);
    named.label = Some("Area".into());
    let mut blank = group("blank", 0.0, 0.0, 10.0, 10.0);
    blank.label = Some(String::new());
    let mut labelled_card = card("card", 0.0, 0.0, 10.0, 10.0);
    labelled_card.label = Some("Not a group".into());
    let mut second = group("second", 0.0, 0.0, 10.0, 10.0);
    second.label = Some("Zone".into());
    let scene = Scene::new(canvas(
        vec![
            named,
            group("plain", 0.0, 0.0, 1.0, 1.0),
            blank,
            labelled_card,
            second,
        ],
        vec![],
    ));
    let targets: Vec<&str> = scene
        .jump_targets()
        .into_iter()
        .map(|n| n.id.as_str())
        .collect();
    assert_eq!(targets, ["named", "second"]);
}

// ---- Scene: adding ------------------------------------------------------------------------

#[test]
fn adding_cards_uses_the_default_sizes_and_selects_them() {
    let mut scene = Scene::new(Canvas::default());
    scene.begin();
    let t = scene.add_text_card(Point::new(10.0, 20.0), "hello");
    let text = scene.node(&t).expect("added");
    assert_eq!(text.kind, NodeType::Text);
    assert_eq!(text.text.as_deref(), Some("hello"));
    assert_eq!(
        Rect::of(text),
        Rect::new(10.0, 20.0, DEFAULT_CARD.0, DEFAULT_CARD.1)
    );
    assert_eq!(scene.selection, Selection::only_node(&t));

    let f = scene.add_file_card(Point::new(0.0, 0.0), "Notes/Plan.md", Some("#Goals"));
    let file = scene.node(&f).expect("added");
    assert_eq!(file.kind, NodeType::File);
    assert_eq!(file.file.as_deref(), Some("Notes/Plan.md"));
    assert_eq!(file.subpath.as_deref(), Some("#Goals"));
    assert_eq!((file.width, file.height), DEFAULT_FILE_CARD);
    assert_eq!(
        scene.selection,
        Selection::only_node(&f),
        "the newest is the selection"
    );
    let plain = scene.add_file_card(Point::new(0.0, 0.0), "a.png", None);
    assert_eq!(scene.node(&plain).and_then(|n| n.subpath.as_deref()), None);

    let l = scene.add_link_card(Point::new(0.0, 0.0), "https://example.com");
    let link = scene.node(&l).expect("added");
    assert_eq!(link.kind, NodeType::Link);
    assert_eq!(link.url.as_deref(), Some("https://example.com"));
    scene.commit();
    assert_eq!(scene.canvas.nodes.len(), 4);
    assert_eq!(
        ids(&scene.canvas).last().copied(),
        Some(l.as_str()),
        "added on top"
    );
    assert_eq!(scene.undo_depth(), 1);
    // Ids are fresh and distinct.
    let mut all = vec![t, f, plain, l];
    all.sort();
    all.dedup();
    assert_eq!(all.len(), 4);
}

#[test]
fn add_node_puts_any_node_on_top_and_selects_it() {
    let mut scene = two_cards();
    let id = scene.add_node(card("c", 0.0, 0.0, 1.0, 1.0));
    assert_eq!(id, "c");
    assert_eq!(ids(&scene.canvas), ["a", "b", "c"]);
    assert_eq!(scene.selection, Selection::only_node("c"));
}

#[test]
fn add_group_goes_beneath_every_card_and_gets_the_default_label() {
    let mut scene = Scene::new(canvas(
        vec![
            group("g0", 0.0, 0.0, 10.0, 10.0),
            card("a", 0.0, 0.0, 10.0, 10.0),
            card("b", 0.0, 0.0, 10.0, 10.0),
        ],
        vec![],
    ));
    let g = scene.add_group(Rect::new(1.0, 2.0, 30.0, 40.0), None);
    assert_eq!(ids(&scene.canvas), ["g0", g.as_str(), "a", "b"]);
    let node = scene.node(&g).expect("added");
    assert_eq!(node.kind, NodeType::Group);
    assert_eq!(node.label.as_deref(), Some("Untitled group"));
    assert_eq!(Rect::of(node), Rect::new(1.0, 2.0, 30.0, 40.0));
    assert_eq!(scene.selection, Selection::only_node(&g));

    let mut cards_only = two_cards();
    let g = cards_only.add_group(Rect::new(0.0, 0.0, 1.0, 1.0), Some("Named"));
    assert_eq!(ids(&cards_only.canvas), [g.as_str(), "a", "b"]);
    assert_eq!(
        cards_only.node(&g).and_then(|n| n.label.as_deref()),
        Some("Named")
    );

    let mut empty = Scene::new(Canvas::default());
    let g = empty.add_group(Rect::new(0.0, 0.0, 1.0, 1.0), None);
    assert_eq!(ids(&empty.canvas), [g.as_str()]);
}

#[test]
fn add_edge_records_both_sides_and_selects_the_edge() {
    let mut scene = two_cards();
    let id = scene.add_edge("b", Side::Top, "a", Side::Bottom);
    let e = scene.edge(&id).expect("added");
    assert_eq!((e.from_node.as_str(), e.to_node.as_str()), ("b", "a"));
    assert_eq!(
        (e.from_side, e.to_side),
        (Some(Side::Top), Some(Side::Bottom))
    );
    assert_eq!(
        (e.from_end, e.to_end),
        (None, None),
        "default ends are not written"
    );
    assert_eq!(scene.selection, Selection::only_edge(&id));
    assert_eq!(scene.canvas.edges.len(), 2);
}

// ---- Scene: moving and resizing ---------------------------------------------------------------

#[test]
fn moving_a_group_carries_its_members_exactly_once() {
    let mut scene = Scene::new(canvas(
        vec![
            group("g", 0.0, 0.0, 500.0, 500.0),
            card("m", 100.0, 100.0, 100.0, 100.0),
            card("n", 300.0, 300.0, 100.0, 100.0),
            card("o", 600.0, 600.0, 100.0, 100.0),
        ],
        vec![],
    ));
    // The member is selected as well as its group; it still moves once.
    scene.selection = Selection {
        nodes: vec!["m".into(), "g".into()],
        edges: vec![],
    };
    assert_eq!(scene.carried_nodes(), vec![0, 1, 2]);
    scene.move_selection(10.0, 20.0);
    assert_eq!(rect_of(&scene, "g"), Rect::new(10.0, 20.0, 500.0, 500.0));
    assert_eq!(rect_of(&scene, "m"), Rect::new(110.0, 120.0, 100.0, 100.0));
    assert_eq!(rect_of(&scene, "n"), Rect::new(310.0, 320.0, 100.0, 100.0));
    assert_eq!(
        rect_of(&scene, "o"),
        Rect::new(600.0, 600.0, 100.0, 100.0),
        "outside the group"
    );
    // A selected card alone moves only itself.
    scene.selection = Selection::only_node("o");
    scene.move_selection(-100.0, 0.0);
    assert_eq!(rect_of(&scene, "o").x, 500.0);
    assert_eq!(rect_of(&scene, "g").x, 10.0);
    // Ids the canvas lacks are ignored.
    scene.selection = Selection::only_node("zzz");
    scene.move_selection(1.0, 1.0);
    assert_eq!(rect_of(&scene, "g").x, 10.0);
}

#[test]
fn nudging_lands_every_carried_node_on_the_grid() {
    let mut scene = Scene::new(canvas(
        vec![
            group("g", GRID * 0.3, 0.0, GRID * 20.0, GRID * 20.0),
            card("m", GRID * 1.3, GRID * 2.6, GRID, GRID),
        ],
        vec![],
    ));
    scene.selection = Selection::only_node("g");
    scene.nudge_selection(GRID, 0.0);
    assert_eq!(rect_of(&scene, "g").x, GRID, "0.3 + 1 grid rounds to 1");
    assert_eq!(rect_of(&scene, "m").x, 2.0 * GRID);
    assert_eq!(rect_of(&scene, "m").y, 3.0 * GRID);
    scene.nudge_selection(0.0, -GRID);
    assert_eq!(rect_of(&scene, "m").y, 2.0 * GRID);
}

#[test]
fn resize_node_and_set_node_rect_change_the_geometry() {
    let mut scene = two_cards();
    scene.resize_node("a", Handle::BottomRight, 50.0, 25.0);
    assert_eq!(rect_of(&scene, "a"), Rect::new(0.0, 0.0, 150.0, 125.0));
    scene.resize_node("a", Handle::Left, -1000.0, 0.0);
    assert_eq!(rect_of(&scene, "a"), Rect::new(-1000.0, 0.0, 1150.0, 125.0));
    scene.resize_node("a", Handle::Top, 0.0, 1e6);
    assert_eq!(
        rect_of(&scene, "a").height,
        MIN_NODE.1,
        "the minimum applies"
    );
    scene.set_node_rect("a", Rect::new(1.0, 2.0, 3.0, 4.0));
    assert_eq!(
        rect_of(&scene, "a"),
        Rect::new(1.0, 2.0, 3.0, 4.0),
        "set_node_rect takes the rect as given"
    );
    scene.resize_node("zzz", Handle::Top, 1.0, 1.0);
    scene.set_node_rect("zzz", Rect::new(1.0, 2.0, 3.0, 4.0));
    assert_eq!(scene.canvas.nodes.len(), 2);
}

#[test]
fn snap_selection_snaps_the_selected_nodes_only() {
    let mut scene = Scene::new(canvas(
        vec![
            card("a", GRID * 0.4, GRID * 0.6, 10.0, 10.0),
            card("b", GRID * 0.4, GRID * 0.6, 10.0, 10.0),
        ],
        vec![],
    ));
    scene.selection = Selection::only_node("a");
    scene.snap_selection();
    assert_eq!(rect_of(&scene, "a"), Rect::new(0.0, GRID, 10.0, 10.0));
    assert_eq!(rect_of(&scene, "b").x, GRID * 0.4);
}

// ---- Scene: deleting, colours, labels, ends ------------------------------------------------------------

#[test]
fn deleting_the_selection_takes_the_edges_that_touched_it() {
    let mut scene = Scene::new(canvas(
        vec![
            card("a", 0.0, 0.0, 1.0, 1.0),
            card("b", 0.0, 0.0, 1.0, 1.0),
            card("c", 0.0, 0.0, 1.0, 1.0),
        ],
        vec![
            Edge::new("ab", "a", "b"),
            Edge::new("bc", "b", "c"),
            Edge::new("ca", "c", "a"),
        ],
    ));
    scene.selection = Selection {
        nodes: vec!["a".into()],
        edges: vec!["bc".into()],
    };
    scene.edit(|_| {});
    scene.begin();
    scene.delete_selection();
    scene.commit();
    assert_eq!(ids(&scene.canvas), ["b", "c"]);
    assert!(
        scene.canvas.edges.is_empty(),
        "ab and ca touched a; bc was selected"
    );
    assert!(scene.selection.is_empty());
    scene.undo();
    assert_eq!(scene.canvas.edges.len(), 3);
    // Deleting an edge alone leaves its nodes.
    scene.selection = Selection::only_edge("ab");
    scene.delete_selection();
    assert_eq!(scene.canvas.nodes.len(), 3);
    assert_eq!(scene.canvas.edges.len(), 2);
}

#[test]
fn set_selection_color_colours_nodes_and_edges_and_default_clears() {
    let mut scene = two_cards();
    scene.selection = Selection {
        nodes: vec!["a".into()],
        edges: vec!["ab".into()],
    };
    scene.set_selection_color(Color::Preset(4));
    assert_eq!(scene.node("a").and_then(|n| n.color.as_deref()), Some("4"));
    assert_eq!(scene.edge("ab").and_then(|e| e.color.as_deref()), Some("4"));
    assert_eq!(scene.node("b").and_then(|n| n.color.as_deref()), None);
    scene.set_selection_color(Color::Hex(0x0a0b0c));
    assert_eq!(
        scene.node("a").and_then(|n| n.color.as_deref()),
        Some("#0a0b0c")
    );
    scene.set_selection_color(Color::Default);
    assert_eq!(scene.node("a").and_then(|n| n.color.as_deref()), None);
    assert_eq!(scene.edge("ab").and_then(|e| e.color.as_deref()), None);
    assert!(!scene.to_json().contains("color"));
}

#[test]
fn set_text_replaces_a_cards_text() {
    let mut scene = two_cards();
    scene.set_text("a", "new words");
    assert_eq!(
        scene.node("a").and_then(|n| n.text.as_deref()),
        Some("new words")
    );
    scene.set_text("a", "");
    assert_eq!(
        scene.node("a").and_then(|n| n.text.as_deref()),
        Some(""),
        "a text card always has text"
    );
    scene.set_text("zzz", "x");
}

#[test]
fn an_empty_label_removes_the_key() {
    let mut scene = two_cards();
    let g = scene.add_group(Rect::new(0.0, 0.0, 1.0, 1.0), Some("Area"));
    scene.set_edge_label("ab", "leads to");
    assert_eq!(
        scene.edge("ab").and_then(|e| e.label.as_deref()),
        Some("leads to")
    );
    assert!(scene.to_json().contains("\"label\":\"leads to\""));
    scene.set_edge_label("ab", "");
    assert_eq!(scene.edge("ab").and_then(|e| e.label.as_deref()), None);
    scene.set_group_label(&g, "Renamed");
    assert_eq!(
        scene.node(&g).and_then(|n| n.label.as_deref()),
        Some("Renamed")
    );
    scene.set_group_label(&g, "");
    assert_eq!(scene.node(&g).and_then(|n| n.label.as_deref()), None);
    assert!(!scene.to_json().contains("label"));
}

#[test]
fn set_edge_ends_writes_only_the_non_default_styles() {
    let mut scene = two_cards();
    let json_ends = |scene: &Scene| {
        let json = scene.to_json();
        (json.contains("fromEnd"), json.contains("toEnd"))
    };
    scene.set_edge_ends("ab", false, true);
    assert_eq!(
        scene.edge("ab").map(|e| (e.from_end, e.to_end)),
        Some((None, None))
    );
    assert_eq!(
        json_ends(&scene),
        (false, false),
        "the defaults are not written"
    );
    assert_eq!(scene.edge("ab").map(edge_ends), Some((false, true)));
    scene.set_edge_ends("ab", true, false);
    assert_eq!(
        scene.edge("ab").map(|e| (e.from_end, e.to_end)),
        Some((Some(EndStyle::Arrow), Some(EndStyle::None)))
    );
    assert!(scene
        .to_json()
        .contains("\"fromEnd\":\"arrow\",\"toEnd\":\"none\""));
    assert_eq!(scene.edge("ab").map(edge_ends), Some((true, false)));
    scene.set_edge_ends("ab", true, true);
    assert_eq!(json_ends(&scene), (true, false));
    scene.set_edge_ends("ab", false, false);
    assert_eq!(json_ends(&scene), (false, true));
    assert_eq!(scene.edge("ab").map(edge_ends), Some((false, false)));
}

#[test]
fn flip_edge_swaps_the_ends_with_their_sides() {
    let mut scene = two_cards();
    scene.flip_edge("ab");
    let e = scene.edge("ab").expect("exists");
    assert_eq!((e.from_node.as_str(), e.to_node.as_str()), ("b", "a"));
    assert_eq!(
        (e.from_side, e.to_side),
        (Some(Side::Left), Some(Side::Right))
    );
    scene.canvas.edges[0].to_side = None;
    scene.flip_edge("ab");
    let e = scene.edge("ab").expect("exists");
    assert_eq!((e.from_node.as_str(), e.to_node.as_str()), ("a", "b"));
    assert_eq!(
        (e.from_side, e.to_side),
        (None, Some(Side::Left)),
        "a missing side travels too"
    );
    scene.flip_edge("zzz");
}

#[test]
fn select_all_takes_every_node_and_edge() {
    let mut scene = two_cards();
    scene.select_all();
    assert_eq!(scene.selection.nodes, ["a", "b"]);
    assert_eq!(scene.selection.edges, ["ab"]);
}

#[test]
fn raise_selection_lifts_cards_in_their_order_and_leaves_groups() {
    let mut scene = Scene::new(canvas(
        vec![
            group("g", 0.0, 0.0, 1.0, 1.0),
            card("a", 0.0, 0.0, 1.0, 1.0),
            card("b", 0.0, 0.0, 1.0, 1.0),
            card("c", 0.0, 0.0, 1.0, 1.0),
        ],
        vec![],
    ));
    scene.selection = Selection {
        nodes: vec!["a".into(), "g".into()],
        edges: vec![],
    };
    scene.raise_selection();
    assert_eq!(ids(&scene.canvas), ["g", "b", "c", "a"]);
    scene.selection = Selection {
        nodes: vec!["c".into(), "a".into()],
        edges: vec![],
    };
    scene.raise_selection();
    assert_eq!(
        ids(&scene.canvas),
        ["g", "b", "c", "a"],
        "z-order among the raised is kept: c stays beneath a"
    );
    scene.selection = Selection::only_node("b");
    scene.raise_selection();
    assert_eq!(ids(&scene.canvas), ["g", "c", "a", "b"]);
    scene.selection.clear();
    scene.raise_selection();
    assert_eq!(ids(&scene.canvas), ["g", "c", "a", "b"]);
}

// ---- Scene: groups ------------------------------------------------------------------------------

#[test]
fn align_group_to_members_wraps_them_with_padding() {
    let mut scene = Scene::new(canvas(
        vec![
            group("g", 0.0, 0.0, 1000.0, 1000.0),
            card("m", 100.0, 100.0, 100.0, 100.0),
            card("n", 300.0, 300.0, 100.0, 100.0),
            group("empty", 2000.0, 2000.0, 100.0, 100.0),
        ],
        vec![],
    ));
    scene.align_group_to_members("g", 20.0);
    assert_eq!(rect_of(&scene, "g"), Rect::new(80.0, 80.0, 340.0, 340.0));
    assert_eq!(
        rect_of(&scene, "m"),
        Rect::new(100.0, 100.0, 100.0, 100.0),
        "members do not move"
    );
    scene.align_group_to_members("empty", 20.0);
    assert_eq!(
        rect_of(&scene, "empty"),
        Rect::new(2000.0, 2000.0, 100.0, 100.0)
    );
    scene.align_group_to_members("m", 20.0);
    assert_eq!(
        rect_of(&scene, "m"),
        Rect::new(100.0, 100.0, 100.0, 100.0),
        "a card is not a group"
    );
    scene.align_group_to_members("zzz", 20.0);
}

#[test]
fn group_selection_makes_a_group_around_the_selection() {
    let mut scene = two_cards();
    scene.selection = Selection {
        nodes: vec!["a".into(), "b".into()],
        edges: vec!["ab".into()],
    };
    scene.begin();
    let g = scene.group_selection(30.0).expect("two nodes make a group");
    scene.commit();
    assert_eq!(
        ids(&scene.canvas),
        [g.as_str(), "a", "b"],
        "the group goes beneath"
    );
    assert_eq!(rect_of(&scene, &g), Rect::new(-30.0, -30.0, 460.0, 160.0));
    assert_eq!(
        scene.node(&g).and_then(|n| n.label.as_deref()),
        Some("Untitled group")
    );
    assert_eq!(scene.selection, Selection::only_node(&g));
    assert_eq!(group_members(&scene.canvas, 0), vec![1, 2]);
    scene.undo();
    assert_eq!(ids(&scene.canvas), ["a", "b"]);
    // Nothing to group: no change and no step.
    scene.selection = Selection::only_edge("ab");
    scene.begin();
    assert_eq!(scene.group_selection(30.0), None);
    scene.commit();
    assert_eq!(scene.canvas.nodes.len(), 2);
    assert_eq!(scene.selection, Selection::only_edge("ab"));
}

// ---- Scene: clipboard ------------------------------------------------------------------------------

/// Three cards in a row, a group around the first two, edges a→b and b→c.
fn chain() -> Scene {
    Scene::new(canvas(
        vec![
            group("g", -50.0, -50.0, 500.0, 200.0),
            card("a", 0.0, 0.0, 100.0, 100.0),
            card("b", 300.0, 0.0, 100.0, 100.0),
            card("c", 600.0, 0.0, 100.0, 100.0),
        ],
        vec![
            sided("ab", "a", Side::Right, "b", Side::Left),
            sided("bc", "b", Side::Right, "c", Side::Left),
        ],
    ))
}

#[test]
fn copy_selection_takes_the_nodes_and_the_edges_among_them_with_ids_kept() {
    let mut scene = chain();
    scene.selection = Selection {
        nodes: vec!["b".into(), "a".into()],
        edges: vec!["bc".into()],
    };
    let clip = scene.copy_selection();
    assert_eq!(ids(&clip), ["a", "b"], "z-order, not selection order");
    assert_eq!(clip.edges.len(), 1);
    assert_eq!(
        clip.edges[0].id, "ab",
        "bc leads out of the selection, even though selected"
    );
    assert_eq!(
        clip.nodes[0], scene.canvas.nodes[1],
        "a copy, byte for byte"
    );
    // A selected group brings its members.
    scene.selection = Selection::only_node("g");
    let clip = scene.copy_selection();
    assert_eq!(ids(&clip), ["g", "a", "b"]);
    assert_eq!(clip.edges.len(), 1);
    // Nothing selected, nothing copied; the original is untouched throughout.
    scene.selection.clear();
    assert_eq!(scene.copy_selection().nodes.len(), 0);
    assert_eq!(scene.canvas.nodes.len(), 4);
    assert!(!scene.can_undo());
}

#[test]
fn the_clipboard_text_is_a_canvas_file() {
    let mut scene = chain();
    scene.selection = Selection {
        nodes: vec!["a".into(), "b".into()],
        edges: vec![],
    };
    let text = scene.copy_selection().to_json();
    assert!(text.starts_with("{\n\t\"nodes\":[\n"));
    let back = Canvas::from_json(&text).expect("the clipboard parses as a canvas");
    assert_eq!(back, scene.copy_selection());
}

#[test]
fn paste_gives_fresh_ids_remaps_edges_and_lands_at_the_point() {
    let mut scene = chain();
    scene.selection = Selection {
        nodes: vec!["a".into(), "b".into()],
        edges: vec![],
    };
    let clip = scene.copy_selection();
    scene.begin();
    let pasted = scene.paste(&clip, Some(Point::new(1000.0, 2000.0)));
    scene.commit();
    assert_eq!(pasted.len(), 2);
    assert_eq!(scene.canvas.nodes.len(), 6);
    assert_eq!(
        ids(&scene.canvas)[4..],
        [pasted[0].as_str(), pasted[1].as_str()],
        "appended on top, in clip order"
    );
    assert!(pasted
        .iter()
        .all(|id| id.len() == 16 && id != "a" && id != "b"));
    // The clip's bounding box top-left (0, 0) lands at the point.
    assert_eq!(
        rect_of(&scene, &pasted[0]),
        Rect::new(1000.0, 2000.0, 100.0, 100.0)
    );
    assert_eq!(
        rect_of(&scene, &pasted[1]),
        Rect::new(1300.0, 2000.0, 100.0, 100.0)
    );
    assert_eq!(
        scene.node(&pasted[0]).and_then(|n| n.text.as_deref()),
        Some("a")
    );
    // The edge between them came along, onto the new ids, with its sides.
    assert_eq!(scene.canvas.edges.len(), 3);
    let e = &scene.canvas.edges[2];
    assert_eq!(
        (e.from_node.as_str(), e.to_node.as_str()),
        (pasted[0].as_str(), pasted[1].as_str())
    );
    assert_eq!(
        (e.from_side, e.to_side),
        (Some(Side::Right), Some(Side::Left))
    );
    assert_ne!(e.id, "ab");
    assert_eq!(
        scene.selection,
        Selection {
            nodes: pasted.clone(),
            edges: vec![]
        }
    );
    // The originals are where they were, and one undo removes the paste.
    assert_eq!(rect_of(&scene, "a"), Rect::new(0.0, 0.0, 100.0, 100.0));
    scene.undo();
    assert_eq!(scene.canvas.nodes.len(), 4);
    assert_eq!(scene.canvas.edges.len(), 2);
}

#[test]
fn paste_in_place_steps_one_grid_cell_and_drops_edges_to_nodes_outside_the_clip() {
    let mut scene = chain();
    let mut clip = canvas(
        vec![
            card("x", 100.0, 200.0, 50.0, 50.0),
            card("y", 400.0, 300.0, 50.0, 50.0),
        ],
        vec![Edge::new("xy", "x", "y"), Edge::new("xq", "x", "q")],
    );
    clip.nodes[0].color = Some("3".into());
    let pasted = scene.paste(&clip, None);
    assert_eq!(
        rect_of(&scene, &pasted[0]),
        Rect::new(100.0 + GRID, 200.0 + GRID, 50.0, 50.0)
    );
    assert_eq!(
        rect_of(&scene, &pasted[1]),
        Rect::new(400.0 + GRID, 300.0 + GRID, 50.0, 50.0)
    );
    assert_eq!(
        scene.node(&pasted[0]).and_then(|n| n.color.as_deref()),
        Some("3")
    );
    assert_eq!(scene.canvas.edges.len(), 3, "xq has no q to attach to");
    assert_eq!(scene.canvas.edges[2].to_node, pasted[1]);
    // An empty clip does nothing.
    let before = scene.canvas.clone();
    let selection = scene.selection.clone();
    assert!(scene.paste(&Canvas::default(), None).is_empty());
    assert_eq!(scene.canvas, before);
    assert_eq!(scene.selection, selection);
}

#[test]
fn a_paste_from_the_clipboard_text_matches_the_copy_bar_ids_and_position() {
    let mut scene = chain();
    scene.selection = Selection::only_node("g");
    let text = scene.copy_selection().to_json();
    let clip = Canvas::from_json(&text).expect("parses");
    let pasted = scene.paste(&clip, Some(Point::new(0.0, 1000.0)));
    assert_eq!(pasted.len(), 3);
    // The group's top-left is the clip's top-left.
    assert_eq!(
        rect_of(&scene, &pasted[0]),
        Rect::new(0.0, 1000.0, 500.0, 200.0)
    );
    assert_eq!(
        rect_of(&scene, &pasted[1]),
        Rect::new(50.0, 1050.0, 100.0, 100.0)
    );
    let kinds: Vec<NodeType> = pasted
        .iter()
        .filter_map(|id| scene.node(id))
        .map(|n| n.kind)
        .collect();
    assert_eq!(kinds, [NodeType::Group, NodeType::Text, NodeType::Text]);
    // The pasted group encloses its pasted members, so it moves them again.
    let gi = scene.node_index(&pasted[0]).expect("pasted");
    assert_eq!(group_members(&scene.canvas, gi).len(), 2);
}

#[test]
fn duplicate_selection_copies_with_an_offset_and_selects_the_copies() {
    let mut scene = chain();
    scene.selection = Selection {
        nodes: vec!["a".into(), "b".into()],
        edges: vec!["ab".into()],
    };
    scene.begin();
    let copies = scene.duplicate_selection(Point::new(GRID, GRID));
    scene.commit();
    assert_eq!(copies.len(), 2);
    assert_eq!(scene.canvas.nodes.len(), 6);
    assert_eq!(scene.canvas.edges.len(), 3);
    assert_eq!(
        rect_of(&scene, &copies[0]),
        Rect::new(GRID, GRID, 100.0, 100.0)
    );
    assert_eq!(
        rect_of(&scene, &copies[1]),
        Rect::new(300.0 + GRID, GRID, 100.0, 100.0)
    );
    assert_eq!(
        rect_of(&scene, "a"),
        Rect::new(0.0, 0.0, 100.0, 100.0),
        "the original stays"
    );
    assert_eq!(scene.selection.nodes, copies);
    assert!(scene.selection.edges.is_empty());
    let dup = &scene.canvas.edges[2];
    assert_eq!(
        (dup.from_node.as_str(), dup.to_node.as_str()),
        (copies[0].as_str(), copies[1].as_str())
    );
    scene.undo();
    assert_eq!(scene.canvas.nodes.len(), 4);
    // Nothing selected duplicates nothing.
    scene.selection.clear();
    assert!(scene.duplicate_selection(Point::new(GRID, GRID)).is_empty());
    assert_eq!(scene.canvas.nodes.len(), 4);
}

// ---- round trips through the writer ---------------------------------------------------------------

fn scene_from(text: &str) -> Scene {
    Scene::new(Canvas::from_json(text).expect("the fixture parses"))
}

#[test]
fn a_file_obsidian_wrote_comes_back_byte_identical_without_edits() {
    for (name, text) in [
        ("order", ORDER_OUT),
        ("scrambled", SCRAMBLED_OUT),
        ("noedges", NOEDGES_OUT),
    ] {
        let scene = scene_from(text);
        assert_eq!(scene.to_json(), text, "{name}");
        assert!(!scene.is_dirty());
    }
}

#[test]
fn moving_a_node_writes_what_obsidian_wrote() {
    let mut scene = scene_from(ORDER_IN);
    scene.selection = Selection::only_node("t3");
    scene.begin();
    // t3 goes from (0, 200) to (10, 210), in two steps of one drag.
    scene.move_selection(4.0, 7.0);
    scene.move_selection(6.0, 3.0);
    scene.commit();
    assert_eq!(scene.to_json(), ORDER_OUT);
    // Undone, the file is what Obsidian would have written unedited.
    let unmoved = ORDER_OUT.replace("\"x\":10,\"y\":210", "\"x\":0,\"y\":200");
    assert_ne!(unmoved, ORDER_OUT);
    assert!(scene.undo());
    assert_eq!(scene.to_json(), unmoved);
    assert!(scene.redo());
    assert_eq!(scene.to_json(), ORDER_OUT);
}

#[test]
fn a_move_and_an_undo_leave_obsidians_bytes_alone() {
    for text in [ORDER_OUT, SCRAMBLED_OUT, NOEDGES_OUT] {
        let mut scene = scene_from(text);
        scene.select_all();
        scene.begin();
        scene.move_selection(37.0, -41.0);
        scene.commit();
        assert_ne!(scene.to_json(), text);
        assert!(scene.undo());
        assert_eq!(scene.to_json(), text);
    }
}

#[test]
fn a_set_of_edits_undone_one_by_one_returns_to_the_file() {
    let mut scene = scene_from(ORDER_OUT);
    scene.selection = Selection::only_node("t1");
    scene.edit(|c| c.nodes[2].x = 77.0);
    scene.begin();
    scene.set_selection_color(Color::Preset(1));
    scene.commit();
    scene.begin();
    scene.delete_selection();
    scene.commit();
    scene.begin();
    scene.add_text_card(Point::new(0.0, 0.0), "new");
    scene.commit();
    assert_eq!(scene.undo_depth(), 4);
    while scene.undo() {}
    assert_eq!(scene.to_json(), ORDER_OUT);
}

#[test]
fn control_points_reach_half_the_span_within_obsidians_bounds() {
    // Observed 2026-10-21: two 160 by 60 cards, right side to left side.
    let a = Rect::new(980.0, 0.0, 160.0, 60.0);
    let close_by = Rect::new(1320.0, 50.0, 160.0, 60.0);
    let path = edge_path(&a, Side::Right, &close_by, Side::Left);
    assert!(close(path.from.x, 1147.0) && close(path.to.x, 1313.0));
    assert!(close(path.c1.x, 1233.6833317310773));
    assert!(close(path.c2.x, 1226.3166682689227));
    // Far apart the reach stops at 150; nearly touching it stays at 70.
    let far = edge_path(
        &a,
        Side::Right,
        &Rect::new(1980.0, 50.0, 160.0, 60.0),
        Side::Left,
    );
    assert!(close(far.c1.x, 1147.0 + EDGE_REACH.1));
    let near = edge_path(
        &a,
        Side::Right,
        &Rect::new(1200.0, 0.0, 160.0, 60.0),
        Side::Left,
    );
    assert!(close(near.c1.x, 1147.0 + EDGE_REACH.0));
    assert!(close(near.c2.x, 1193.0 - EDGE_REACH.0));
}
