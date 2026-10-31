//! JSON Canvas 1.0 (`.canvas`), read and written the way Obsidian writes it.
//!
//! Nodes are kept in file order because order is z-order (earlier beneath later).
//! Unknown fields are preserved for plugin data.
//!
//! # What Obsidian writes
//!
//! Observed over CDP on 2026-10-19 (`tools/obsidian-probe/canvas.mjs`, Obsidian
//! 1.14.4 on installer 1.13.7) and written up in `docs/silica/Specification.md`
//! under "Observed canvas write behavior". An edited save rebuilds every object
//! from the model:
//!
//! - A node or edge keeps **the keys of the object it was read from, in file
//!   order**, minus the ones the model owns. Those come back after them in a
//!   fixed order: for a node `id, x, y, width, height, color, type` and then
//!   the type's own keys (`text`; `file, subpath`; `url`; `label, background,
//!   backgroundStyle`); for an edge `id, fromNode, fromSide, toNode, toSide,
//!   fromEnd, toEnd, color, label`. A key already present is overwritten in
//!   place, so a file Obsidian wrote comes back in the same order, and a node
//!   made in the app (no file order) is written in exactly that fixed order.
//! - The model owns `x, y, width, height, color` on every node, `label` and
//!   `background` on a group as well (not `backgroundStyle`), and everything
//!   but `id` and unknown keys on an edge. `fromEnd: none` and `toEnd: arrow`
//!   are defaults and are not written.
//! - Reading a file puts the groups first and keeps the rest in file order;
//!   nodes are written in that order, a node added later at the end.
//! - The top-level object keeps its own key order; `nodes` then `edges` are
//!   appended when absent.
//! - The formatter is [`format_json`]: tabs, no spaces, no trailing newline,
//!   and a container stays on one line unless one of its values is `null`, an
//!   object or an array.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Default)]
pub struct Canvas {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub extra: Map<String, Value>,
    /// Top-level keys of the file this was read from, in file order.
    key_order: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeType {
    Text,
    File,
    Link,
    Group,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackgroundStyle {
    Cover,
    Ratio,
    Repeat,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub id: String,
    pub kind: NodeType,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// `"1"`..`"6"` preset or `#rrggbb`.
    pub color: Option<String>,
    // text
    pub text: Option<String>,
    // file
    pub file: Option<String>,
    pub subpath: Option<String>,
    // link
    pub url: Option<String>,
    // group
    pub label: Option<String>,
    pub background: Option<String>,
    pub background_style: Option<BackgroundStyle>,
    pub extra: Map<String, Value>,
    /// Keys of the object this node was read from, in file order. Empty for
    /// a node made in Silica.
    key_order: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EndStyle {
    None,
    Arrow,
}

#[derive(Debug, Clone)]
pub struct Edge {
    pub id: String,
    pub from_node: String,
    pub from_side: Option<Side>,
    /// Defaults to `none`, which is not written.
    pub from_end: Option<EndStyle>,
    pub to_node: String,
    pub to_side: Option<Side>,
    /// Defaults to `arrow`, which is not written.
    pub to_end: Option<EndStyle>,
    pub color: Option<String>,
    pub label: Option<String>,
    pub extra: Map<String, Value>,
    /// Keys of the object this edge was read from, in file order.
    key_order: Vec<String>,
}

// ---- the serde shapes ------------------------------------------------------
// The public types carry the file's key order, which serde cannot derive, so
// each is read through a derived twin and written by hand.

#[derive(Serialize, Deserialize)]
struct CanvasFields {
    #[serde(default)]
    nodes: Vec<Node>,
    #[serde(default)]
    edges: Vec<Edge>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

#[derive(Deserialize)]
struct NodeFields {
    id: String,
    #[serde(rename = "type")]
    kind: NodeType,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    subpath: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    background: Option<String>,
    #[serde(default, rename = "backgroundStyle")]
    background_style: Option<BackgroundStyle>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

#[derive(Deserialize)]
struct EdgeFields {
    id: String,
    #[serde(rename = "fromNode")]
    from_node: String,
    #[serde(default, rename = "fromSide")]
    from_side: Option<Side>,
    #[serde(default, rename = "fromEnd")]
    from_end: Option<EndStyle>,
    #[serde(rename = "toNode")]
    to_node: String,
    #[serde(default, rename = "toSide")]
    to_side: Option<Side>,
    #[serde(default, rename = "toEnd")]
    to_end: Option<EndStyle>,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    label: Option<String>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

/// Reads an object as `T` and hands back its keys in file order as well.
fn object_with_order<'de, D, T>(deserializer: D) -> Result<(T, Vec<String>), D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let map = Map::<String, Value>::deserialize(deserializer)?;
    let order = map.keys().cloned().collect();
    let fields = serde_json::from_value(Value::Object(map)).map_err(serde::de::Error::custom)?;
    Ok((fields, order))
}

impl<'de> Deserialize<'de> for Canvas {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let (f, key_order): (CanvasFields, _) = object_with_order(deserializer)?;
        // Obsidian loads the groups first and the rest in file order; a node
        // made afterwards goes on the end. Nodes are then written in that order.
        let (groups, rest): (Vec<_>, Vec<_>) =
            f.nodes.into_iter().partition(|n| n.kind == NodeType::Group);
        Ok(Canvas {
            nodes: groups.into_iter().chain(rest).collect(),
            edges: f.edges,
            extra: f.extra,
            key_order,
        })
    }
}

impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let (f, key_order): (NodeFields, _) = object_with_order(deserializer)?;
        Ok(Node {
            id: f.id,
            kind: f.kind,
            x: f.x,
            y: f.y,
            width: f.width,
            height: f.height,
            color: f.color,
            text: f.text,
            file: f.file,
            subpath: f.subpath,
            url: f.url,
            label: f.label,
            background: f.background,
            background_style: f.background_style,
            extra: f.extra,
            key_order,
        })
    }
}

impl<'de> Deserialize<'de> for Edge {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let (f, key_order): (EdgeFields, _) = object_with_order(deserializer)?;
        Ok(Edge {
            id: f.id,
            from_node: f.from_node,
            from_side: f.from_side,
            from_end: f.from_end,
            to_node: f.to_node,
            to_side: f.to_side,
            to_end: f.to_end,
            color: f.color,
            label: f.label,
            extra: f.extra,
            key_order,
        })
    }
}

/// Builds an object the way Obsidian's `getData` does: the keys the file had,
/// in file order, except the ones the model owns (`owned`); then the model's
/// keys in its fixed order, each overwriting in place when the file already
/// had it and appended otherwise.
struct OrderedObject {
    map: Map<String, Value>,
}

impl OrderedObject {
    fn new(key_order: &[String], owned: &[&str], value_of: impl Fn(&str) -> Option<Value>) -> Self {
        let mut map = Map::new();
        for key in key_order {
            if owned.contains(&key.as_str()) {
                continue;
            }
            if let Some(v) = value_of(key) {
                map.insert(key.clone(), v);
            }
        }
        OrderedObject { map }
    }

    fn set(&mut self, key: &str, value: Option<Value>) {
        if let Some(v) = value {
            self.map.insert(key.to_owned(), v);
        } else {
            self.map.shift_remove(key);
        }
    }
}

fn enum_value<T: Serialize>(v: &Option<T>) -> Option<Value> {
    v.as_ref()
        .map(|x| serde_json::to_value(x).expect("a unit enum serializes"))
}
fn str_value(v: &Option<String>) -> Option<Value> {
    v.as_ref().map(|s| Value::String(s.clone()))
}
fn num_value(v: f64) -> Option<Value> {
    Some(serde_json::Number::from_f64(v).map_or(Value::Null, Value::Number))
}

impl Node {
    /// A node made in Silica: written in the order Obsidian writes a node it
    /// made itself.
    pub fn new(
        id: impl Into<String>,
        kind: NodeType,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> Self {
        Node {
            id: id.into(),
            kind,
            x,
            y,
            width,
            height,
            color: None,
            text: None,
            file: None,
            subpath: None,
            url: None,
            label: None,
            background: None,
            background_style: None,
            extra: Map::new(),
            key_order: Vec::new(),
        }
    }

    /// Keys of the object this node was read from, in file order.
    pub fn key_order(&self) -> &[String] {
        &self.key_order
    }

    fn typed_value(&self, key: &str) -> Option<Value> {
        Some(match key {
            "id" => Value::String(self.id.clone()),
            "type" => serde_json::to_value(self.kind).expect("a unit enum serializes"),
            "x" => return num_value(self.x),
            "y" => return num_value(self.y),
            "width" => return num_value(self.width),
            "height" => return num_value(self.height),
            "color" => return str_value(&self.color),
            "text" => return str_value(&self.text),
            "file" => return str_value(&self.file),
            "subpath" => return str_value(&self.subpath),
            "url" => return str_value(&self.url),
            "label" => return str_value(&self.label),
            "background" => return str_value(&self.background),
            "backgroundStyle" => return enum_value(&self.background_style),
            other => return self.extra.get(other).cloned(),
        })
    }

    fn to_object(&self) -> Map<String, Value> {
        let owned: &[&str] = match self.kind {
            NodeType::Group => &["x", "y", "width", "height", "color", "label", "background"],
            _ => &["x", "y", "width", "height", "color"],
        };
        let mut o = OrderedObject::new(&self.key_order, owned, |k| self.typed_value(k));
        for key in ["id", "x", "y", "width", "height", "color", "type"] {
            o.set(key, self.typed_value(key));
        }
        let own: &[&str] = match self.kind {
            NodeType::Text => &["text"],
            NodeType::File => &["file", "subpath"],
            NodeType::Link => &["url"],
            NodeType::Group => &["label", "background", "backgroundStyle"],
        };
        for key in own {
            o.set(key, self.typed_value(key));
        }
        // A text node always carries `text`, even empty.
        if self.kind == NodeType::Text && self.text.is_none() {
            o.set("text", Some(Value::String(String::new())));
        }
        o.map
    }
}

impl Edge {
    /// An edge made in Silica.
    pub fn new(
        id: impl Into<String>,
        from_node: impl Into<String>,
        to_node: impl Into<String>,
    ) -> Self {
        Edge {
            id: id.into(),
            from_node: from_node.into(),
            from_side: None,
            from_end: None,
            to_node: to_node.into(),
            to_side: None,
            to_end: None,
            color: None,
            label: None,
            extra: Map::new(),
            key_order: Vec::new(),
        }
    }

    /// Keys of the object this edge was read from, in file order.
    pub fn key_order(&self) -> &[String] {
        &self.key_order
    }

    fn typed_value(&self, key: &str) -> Option<Value> {
        match key {
            "id" => Some(Value::String(self.id.clone())),
            "fromNode" => Some(Value::String(self.from_node.clone())),
            "fromSide" => enum_value(&self.from_side),
            "fromEnd" => enum_value(&self.from_end.filter(|e| *e != EndStyle::None)),
            "toNode" => Some(Value::String(self.to_node.clone())),
            "toSide" => enum_value(&self.to_side),
            "toEnd" => enum_value(&self.to_end.filter(|e| *e != EndStyle::Arrow)),
            "color" => str_value(&self.color),
            "label" => str_value(&self.label),
            other => self.extra.get(other).cloned(),
        }
    }

    fn to_object(&self) -> Map<String, Value> {
        const OWNED: &[&str] = &[
            "fromNode", "fromSide", "toNode", "toSide", "fromEnd", "toEnd", "color", "label",
        ];
        let mut o = OrderedObject::new(&self.key_order, OWNED, |k| self.typed_value(k));
        for key in [
            "id", "fromNode", "fromSide", "toNode", "toSide", "fromEnd", "toEnd", "color", "label",
        ] {
            o.set(key, self.typed_value(key));
        }
        o.map
    }
}

impl Serialize for Node {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_object().serialize(serializer)
    }
}

impl Serialize for Edge {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_object().serialize(serializer)
    }
}

impl Serialize for Canvas {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_object().serialize(serializer)
    }
}

impl Canvas {
    pub fn from_json(text: &str) -> serde_json::Result<Canvas> {
        serde_json::from_str(text)
    }

    /// Top-level keys of the file this was read from, in file order.
    pub fn key_order(&self) -> &[String] {
        &self.key_order
    }

    fn to_object(&self) -> Map<String, Value> {
        let nodes = || {
            Some(Value::Array(
                self.nodes
                    .iter()
                    .map(|n| Value::Object(n.to_object()))
                    .collect(),
            ))
        };
        let edges = || {
            Some(Value::Array(
                self.edges
                    .iter()
                    .map(|e| Value::Object(e.to_object()))
                    .collect(),
            ))
        };
        let mut o = OrderedObject::new(&self.key_order, &[], |k| match k {
            "nodes" => nodes(),
            "edges" => edges(),
            other => self.extra.get(other).cloned(),
        });
        o.set("nodes", nodes());
        o.set("edges", edges());
        o.map
    }

    /// The bytes Obsidian would write for this canvas on an edited save: see
    /// the module documentation. No trailing newline.
    pub fn to_json(&self) -> String {
        format_json(&Value::Object(self.to_object()))
    }
}

impl PartialEq for Canvas {
    fn eq(&self, other: &Self) -> bool {
        self.nodes == other.nodes && self.edges == other.edges && self.extra == other.extra
    }
}

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.kind == other.kind
            && self.x == other.x
            && self.y == other.y
            && self.width == other.width
            && self.height == other.height
            && self.color == other.color
            && self.text == other.text
            && self.file == other.file
            && self.subpath == other.subpath
            && self.url == other.url
            && self.label == other.label
            && self.background == other.background
            && self.background_style == other.background_style
            && self.extra == other.extra
    }
}

impl PartialEq for Edge {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.from_node == other.from_node
            && self.from_side == other.from_side
            && self.from_end == other.from_end
            && self.to_node == other.to_node
            && self.to_side == other.to_side
            && self.to_end == other.to_end
            && self.color == other.color
            && self.label == other.label
            && self.extra == other.extra
    }
}

// ---- the formatter ---------------------------------------------------------

/// Formats a JSON value the way Obsidian formats a `.canvas` file: tab
/// indentation, no space after `:` or `,`, no trailing newline. A container
/// is written on one line unless one of its values is `null`, an object or
/// an array; then every value goes on its own line, one indent deeper, and
/// the closing bracket on its own line at the container's indent. Strings
/// and numbers are written as `JSON.stringify` writes them.
pub fn format_json(value: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, value, 0);
    out
}

/// `typeof v === 'object'` in JavaScript, which is what expands a container.
fn is_object_like(v: &Value) -> bool {
    matches!(v, Value::Null | Value::Array(_) | Value::Object(_))
}

fn write_value(out: &mut String, value: &Value, depth: usize) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => write_number(out, n),
        Value::String(s) => out.push_str(&serde_json::to_string(s).expect("a string serializes")),
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
            } else if items.iter().any(is_object_like) {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    out.push_str(if i == 0 { "\n" } else { ",\n" });
                    indent(out, depth + 1);
                    write_value(out, item, depth + 1);
                }
                out.push('\n');
                indent(out, depth);
                out.push(']');
            } else {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_value(out, item, depth + 1);
                }
                out.push(']');
            }
        }
        Value::Object(map) => {
            if map.is_empty() {
                out.push_str("{}");
            } else if map.values().any(is_object_like) {
                out.push('{');
                for (i, (k, v)) in map.iter().enumerate() {
                    out.push_str(if i == 0 { "\n" } else { ",\n" });
                    indent(out, depth + 1);
                    write_key(out, k);
                    write_value(out, v, depth + 1);
                }
                out.push('\n');
                indent(out, depth);
                out.push('}');
            } else {
                out.push('{');
                for (i, (k, v)) in map.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_key(out, k);
                    write_value(out, v, depth + 1);
                }
                out.push('}');
            }
        }
    }
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push('\t');
    }
}

fn write_key(out: &mut String, key: &str) {
    out.push_str(&serde_json::to_string(key).expect("a string serializes"));
    out.push(':');
}

/// A number as `JSON.stringify` writes it: an integral value without a
/// fraction (`1000`, not `1000.0`; `-0` as `0`), otherwise the shortest
/// decimal that reads back to the same double. Exponent forms, which
/// JavaScript uses from 1e21 and below 1e-6, are not reproduced.
fn write_number(out: &mut String, n: &serde_json::Number) {
    if let Some(i) = n.as_i64() {
        let _ = write!(out, "{i}");
    } else if let Some(u) = n.as_u64() {
        let _ = write!(out, "{u}");
    } else if let Some(f) = n.as_f64() {
        if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e21 {
            let _ = write!(out, "{}", f as i128);
        } else if f.is_finite() {
            let _ = write!(out, "{f}");
        } else {
            out.push_str("null");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r##"{
	"nodes":[
		{"id":"a1","type":"text","x":0,"y":0,"width":250,"height":60,"text":"Hello","color":"1"},
		{"id":"b2","type":"file","x":300,"y":0,"width":400,"height":400,"file":"Notes/Plan.md","subpath":"#Goals"},
		{"id":"c3","type":"link","x":0,"y":200,"width":300,"height":200,"url":"https://example.com"},
		{"id":"g4","type":"group","x":-20,"y":-20,"width":800,"height":700,"label":"Area","backgroundStyle":"cover","pluginData":{"k":1}}
	],
	"edges":[
		{"id":"e1","fromNode":"a1","fromSide":"right","toNode":"b2","toSide":"left","toEnd":"arrow","label":"leads to"}
	]
}"##;

    #[test]
    fn parses_all_node_types_and_keeps_unknown_fields() {
        let canvas = Canvas::from_json(SAMPLE).unwrap();
        assert_eq!(canvas.nodes.len(), 4);
        // The group is read first, the rest keep their order.
        assert_eq!(canvas.nodes[0].kind, NodeType::Group);
        assert_eq!(
            canvas.nodes[0].background_style,
            Some(BackgroundStyle::Cover)
        );
        assert_eq!(canvas.nodes[0].extra["pluginData"]["k"], 1);
        assert_eq!(canvas.nodes[1].kind, NodeType::Text);
        assert_eq!(canvas.nodes[2].subpath.as_deref(), Some("#Goals"));
        assert_eq!(canvas.edges[0].to_end, Some(EndStyle::Arrow));
        assert_eq!(canvas.nodes[1].key_order()[..3], ["id", "type", "x"]);
    }

    #[test]
    fn round_trips_through_json() {
        let canvas = Canvas::from_json(SAMPLE).unwrap();
        let text = canvas.to_json();
        assert!(text.starts_with("{\n\t\"nodes\""));
        // The group moves first on reading and the default `toEnd` is
        // dropped; what is written is then a fixed point.
        let again = Canvas::from_json(&text).unwrap();
        let ids = |c: &Canvas| c.nodes.iter().map(|n| n.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&again), ["g4", "a1", "b2", "c3"]);
        assert_eq!(again.edges[0].to_end, None);
        assert_eq!(again.to_json(), text);
    }

    #[test]
    fn a_fractional_position_parses_and_writes_like_javascript() {
        let canvas =
            Canvas::from_json(r#"{"nodes":[{"id":"n","type":"text","text":"","x":0.5,"y":-0,"width":1e3,"height":2.50}]}"#)
                .unwrap();
        assert_eq!(canvas.nodes[0].x, 0.5);
        assert_eq!(
            canvas.to_json(),
            "{\n\t\"nodes\":[\n\t\t{\"id\":\"n\",\"type\":\"text\",\"text\":\"\",\"x\":0.5,\"y\":0,\"width\":1000,\"height\":2.5}\n\t],\n\t\"edges\":[]\n}"
        );
    }

    #[test]
    fn a_node_made_here_is_written_in_obsidians_own_order() {
        let mut canvas = Canvas::default();
        let mut text = Node::new("t", NodeType::Text, 0.0, 0.0, 250.0, 60.0);
        text.text = Some("T".into());
        text.color = Some("3".into());
        let mut group = Node::new("g", NodeType::Group, -20.0, -20.0, 900.0, 800.0);
        group.label = Some("Area".into());
        canvas.nodes.push(text);
        canvas.nodes.push(group);
        canvas.edges.push(Edge::new("e", "t", "g"));
        assert_eq!(
            canvas.to_json(),
            "{\n\t\"nodes\":[\n\t\t{\"id\":\"t\",\"x\":0,\"y\":0,\"width\":250,\"height\":60,\"color\":\"3\",\"type\":\"text\",\"text\":\"T\"},\n\t\t{\"id\":\"g\",\"x\":-20,\"y\":-20,\"width\":900,\"height\":800,\"type\":\"group\",\"label\":\"Area\"}\n\t],\n\t\"edges\":[\n\t\t{\"id\":\"e\",\"fromNode\":\"t\",\"toNode\":\"g\"}\n\t]\n}"
        );
    }

    #[test]
    fn default_edge_ends_are_dropped_and_others_kept() {
        let canvas = Canvas::from_json(
            r#"{"nodes":[],"edges":[{"id":"e","fromNode":"a","toNode":"b","fromEnd":"none","toEnd":"arrow"},{"id":"f","fromNode":"a","toNode":"b","fromEnd":"arrow","toEnd":"none"}]}"#,
        )
        .unwrap();
        assert_eq!(
            canvas.to_json(),
            "{\n\t\"nodes\":[],\n\t\"edges\":[\n\t\t{\"id\":\"e\",\"fromNode\":\"a\",\"toNode\":\"b\"},\n\t\t{\"id\":\"f\",\"fromNode\":\"a\",\"toNode\":\"b\",\"fromEnd\":\"arrow\",\"toEnd\":\"none\"}\n\t]\n}"
        );
    }

    #[test]
    fn the_formatter_expands_only_around_object_like_values() {
        let v: Value = serde_json::from_str(
            r#"{"a":{"k":1},"b":[1,"x",true],"c":[null],"d":{"e":null},"f":[[1]],"g":{},"h":[]}"#,
        )
        .unwrap();
        assert_eq!(
            format_json(&v),
            "{\n\t\"a\":{\"k\":1},\n\t\"b\":[1,\"x\",true],\n\t\"c\":[\n\t\tnull\n\t],\n\t\"d\":{\n\t\t\"e\":null\n\t},\n\t\"f\":[\n\t\t[1]\n\t],\n\t\"g\":{},\n\t\"h\":[]\n}"
        );
    }
}
