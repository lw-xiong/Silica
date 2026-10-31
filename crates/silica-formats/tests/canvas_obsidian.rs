//! The canvas writer against what Obsidian 1.14.4 wrote, observed over CDP on
//! 2026-10-19 (`tools/obsidian-probe/canvas.mjs`, raw run in
//! `tools/obsidian-probe/results/2026-10-19-canvas.json`). Each `*.in.canvas`
//! under `tests/canvas` is the file the probe wrote into a scratch vault, and
//! the `*.out.canvas` beside it is what Obsidian wrote back. The edits the
//! probe made in the app are repeated here on the parsed canvas, and the
//! bytes have to match.

use std::path::PathBuf;

use silica_formats::canvas::{format_json, Canvas, Edge, Node, NodeType, Side};

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/canvas")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Every node and edge type with every optional key, scrambled: the probe's
/// drag added one edge (its id is the one Obsidian generated), and the save
/// rebuilt every object in Obsidian's order around the file's own.
#[test]
fn an_edited_save_keeps_file_keys_in_place_and_appends_the_models() {
    let mut canvas = Canvas::from_json(&fixture("scrambled.in.canvas")).unwrap();
    let mut edge = Edge::new("7ef5a6ac5cffca99", "t1", "f1");
    edge.from_side = Some(Side::Top);
    edge.to_side = Some(Side::Left);
    canvas.edges.push(edge);
    assert_eq!(canvas.to_json(), fixture("scrambled.out.canvas"));
}

/// Groups interleaved with text nodes, then one node moved: groups come
/// first, the rest keep their order, and an edge with all four optional
/// keys shows where they go.
#[test]
fn groups_are_written_first_and_edge_options_in_order() {
    let mut canvas = Canvas::from_json(&fixture("order.in.canvas")).unwrap();
    let t3 = canvas.nodes.iter_mut().find(|n| n.id == "t3").unwrap();
    t3.x = 10.0;
    t3.y = 210.0;
    assert_eq!(canvas.to_json(), fixture("order.out.canvas"));
}

/// A file with no `edges` key: the node moved and an edge added (the probe's
/// drag made one), and `edges` is appended after `nodes`.
#[test]
fn edges_are_appended_when_the_file_had_none() {
    let mut canvas = Canvas::from_json(&fixture("noedges.in.canvas")).unwrap();
    canvas.nodes[0].x = 40.0;
    canvas.nodes[0].y = 30.0;
    let mut edge = Edge::new("e37fb57204c4107d", "only", "f772428526c489f6");
    edge.from_side = Some(Side::Top);
    edge.to_side = Some(Side::Bottom);
    canvas.edges.push(edge);
    assert_eq!(canvas.to_json(), fixture("noedges.out.canvas"));
}

/// A canvas made by the `canvas:new-file` command with one text node typed
/// into it.
#[test]
fn a_new_canvas_with_one_node() {
    let mut canvas = Canvas::default();
    let mut node = Node::new("d143806d1e8b9dd4", NodeType::Text, 10.0, 20.0, 250.0, 60.0);
    node.text = Some("Typed".into());
    canvas.nodes.push(node);
    assert_eq!(canvas.to_json(), fixture("new.out.canvas"));
}

/// Nodes of every type created through the app's own API: the key order a
/// node has when there is no file order to keep.
#[test]
fn nodes_made_in_the_app_are_written_in_the_fixed_order() {
    let mut canvas = Canvas::default();
    let mut text = Node::new("ec6b827235143095", NodeType::Text, 0.0, 0.0, 250.0, 60.0);
    text.text = Some("T".into());
    let mut file = Node::new("a58c9deb305943aa", NodeType::File, 300.0, 0.0, 400.0, 400.0);
    file.file = Some("Plan.md".into());
    file.subpath = Some("#Goals".into());
    let mut link = Node::new("98d36267336fbd1d", NodeType::Link, 0.0, 200.0, 300.0, 200.0);
    link.url = Some("https://example.com".into());
    let mut group = Node::new(
        "f83dfd9109625aca",
        NodeType::Group,
        -20.0,
        -20.0,
        900.0,
        800.0,
    );
    group.label = Some("Area".into());
    let mut plain = Node::new(
        "a710e1fd7024a972",
        NodeType::Group,
        1000.0,
        1000.0,
        100.0,
        100.0,
    );
    plain.label = Some("Untitled group".into());
    let mut colored = Node::new("716ed934c420f719", NodeType::Text, 0.0, 400.0, 250.0, 60.0);
    colored.text = Some("Colored".into());
    colored.color = Some("3".into());
    // Created text, file, link, group, group, text, and written in that
    // order: only reading a file moves the groups first.
    canvas.nodes = vec![text, file, link, group, plain, colored];
    assert_eq!(canvas.to_json(), fixture("api.out.canvas"));
}

/// Opened and closed without an edit, Obsidian reformats the file's own data
/// without rebuilding it: the formatter alone, over every nesting shape, with
/// JavaScript's numbers and string escapes.
#[test]
fn the_formatter_matches_an_unedited_close() {
    let value: serde_json::Value = serde_json::from_str(&fixture("formatter.in.canvas")).unwrap();
    assert_eq!(format_json(&value), fixture("formatter.out.canvas"));
}

/// What Obsidian wrote for a file it had read comes back byte-identical:
/// that order is a fixed point.
#[test]
fn obsidians_output_for_a_read_file_round_trips_byte_identical() {
    for name in [
        "scrambled.out.canvas",
        "order.out.canvas",
        "noedges.out.canvas",
    ] {
        let text = fixture(name);
        let canvas = Canvas::from_json(&text).unwrap();
        assert_eq!(canvas.to_json(), text, "{name}");
    }
}

/// The order Obsidian gives a node it made itself is not a fixed point: read
/// back and saved after an edit, `type` and the type's keys stay where they
/// were and the geometry moves after them. The gate vault's canvas showed
/// exactly this on 2026-10-18: authored `id, x, y, width, height, type, text,
/// color`, it came back `id, type, text, x, y, width, height, color`.
#[test]
fn a_node_made_in_the_app_is_reordered_once_read_back() {
    let canvas = Canvas::from_json(&fixture("new.out.canvas")).unwrap();
    assert_eq!(
        canvas.to_json(),
        "{\n\t\"nodes\":[\n\t\t{\"id\":\"d143806d1e8b9dd4\",\"type\":\"text\",\"text\":\"Typed\",\"x\":10,\"y\":20,\"width\":250,\"height\":60}\n\t],\n\t\"edges\":[]\n}"
    );
}
