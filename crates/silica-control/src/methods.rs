//! The protocol as data.
//!
//! [`METHODS`] is the one place a method, its params and its help text are
//! written down. The app answers `protocol` with [`protocol_json`], the MCP
//! server turns each entry into a tool schema with [`params_schema`], and the
//! docs test reads the same table — so a method cannot exist in one of those
//! three and be missing from the others. Summaries are help text: they are
//! what a client that has never seen Silica reads before calling.

use serde_json::{json, Value};

/// One method of the control protocol.
pub struct MethodSpec {
    pub name: &'static str,
    pub summary: &'static str,
    pub params: &'static [ParamSpec],
}

/// One param of one method.
pub struct ParamSpec {
    pub name: &'static str,
    pub ty: ParamType,
    pub required: bool,
    pub summary: &'static str,
}

/// The JSON types a param may have. Anything richer is a string with its
/// allowed values spelled out in the summary, because a client that guesses
/// wrong should get an error message naming the alternatives rather than a
/// schema violation it cannot read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamType {
    Str,
    Int,
    Num,
    Bool,
    StrList,
}

impl ParamType {
    /// The name used in the `protocol` method's table.
    pub fn as_str(self) -> &'static str {
        match self {
            ParamType::Str => "string",
            ParamType::Int => "integer",
            ParamType::Num => "number",
            ParamType::Bool => "boolean",
            ParamType::StrList => "string[]",
        }
    }

    /// This type as a JSON Schema property.
    pub fn schema(self) -> Value {
        match self {
            ParamType::Str => json!({"type": "string"}),
            ParamType::Int => json!({"type": "integer"}),
            ParamType::Num => json!({"type": "number"}),
            ParamType::Bool => json!({"type": "boolean"}),
            ParamType::StrList => json!({"type": "array", "items": {"type": "string"}}),
        }
    }
}

const fn p(name: &'static str, ty: ParamType, required: bool, summary: &'static str) -> ParamSpec {
    ParamSpec {
        name,
        ty,
        required,
        summary,
    }
}

const fn m(name: &'static str, summary: &'static str, params: &'static [ParamSpec]) -> MethodSpec {
    MethodSpec {
        name,
        summary,
        params,
    }
}

const NONE: &[ParamSpec] = &[];

/// Every method, in the order the specification lists them.
pub const METHODS: &[MethodSpec] = &[
    m(
        "ping",
        "Check the socket is answering; returns the protocol version and the app's version.",
        NONE,
    ),
    m(
        "protocol",
        "List every method with its params and help text, the way this table holds them.",
        NONE,
    ),
    m(
        "wait_loaded",
        "Block until the vault has finished scanning and indexing, or until the timeout.",
        &[p(
            "timeout_ms",
            ParamType::Int,
            false,
            "How long to wait before answering `timed_out`. Default 10000.",
        )],
    ),
    m(
        "wait_idle",
        "Block until the window has painted some quiet frames and the editor has stopped parsing, \
         so that a following `state` or screenshot sees the finished view.",
        &[
            p(
                "frames",
                ParamType::Int,
                false,
                "How many consecutive frames with no pending work to wait for. Default 2.",
            ),
            p(
                "timeout_ms",
                ParamType::Int,
                false,
                "How long to wait before answering `timed_out`. Default 2000. Frame callbacks \
                 stop while the monitors sleep, so this always has a bound.",
            ),
        ],
    ),
    m(
        "state",
        "Dump the whole window as JSON: vault, open file, docks, modal, editor, reading view, \
         graph, canvas, status bar, focus and window size.",
        &[
            p(
                "text",
                ParamType::Bool,
                false,
                "Include the open note's full text in `editor.text`. Default false.",
            ),
            p(
                "rows",
                ParamType::Int,
                false,
                "Truncate every list in the dump to this many entries; a truncated list gets a \
                 sibling `*_total` count. Default 50.",
            ),
        ],
    ),
    m(
        "open",
        "Open a note by name, the way a wikilink does, and return the file now current.",
        &[
            p(
                "name",
                ParamType::Str,
                true,
                "A link target: `Note`, `Note#Heading` or `Note#^block-id`.",
            ),
            p(
                "mode",
                ParamType::Str,
                false,
                "View to open it in: `reading`, `live`, `source` or `graph`. Default: the vault's \
                 own default.",
            ),
        ],
    ),
    m(
        "open_path",
        "Open a file by its vault-relative path, and return the file now current.",
        &[
            p(
                "path",
                ParamType::Str,
                true,
                "Vault-relative path with `/` separators, original case, extension included.",
            ),
            p(
                "mode",
                ParamType::Str,
                false,
                "View to open it in: `reading`, `live`, `source` or `graph`.",
            ),
        ],
    ),
    m(
        "set_mode",
        "Switch the current view without changing the open file.",
        &[p(
            "mode",
            ParamType::Str,
            true,
            "`reading`, `live`, `source` or `graph`.",
        )],
    ),
    m(
        "command",
        "Run a command by its Obsidian command id and return the status bar text it left.",
        &[p(
            "id",
            ParamType::Str,
            true,
            "A command id such as `editor:toggle-bold`; `list_commands` has them all.",
        )],
    ),
    m(
        "keys",
        "Dispatch keystrokes at whatever is focused, in order, and say which were handled.",
        &[p(
            "keys",
            ParamType::StrList,
            true,
            "GPUI keystrokes, e.g. `[\"ctrl-shift-f\", \"enter\"]`.",
        )],
    ),
    m(
        "type",
        "Type text into whatever is focused: the editor, a modal's input, or nothing.",
        &[p(
            "text",
            ParamType::Str,
            true,
            "The text to insert at the cursor. Newlines insert lines.",
        )],
    ),
    m(
        "click",
        "Click at a point: a mouse move, then a press and a release.",
        &[
            p(
                "x",
                ParamType::Num,
                true,
                "Window-local logical pixels from the left edge.",
            ),
            p(
                "y",
                ParamType::Num,
                true,
                "Window-local logical pixels from the top edge.",
            ),
            p(
                "button",
                ParamType::Str,
                false,
                "`left`, `right` or `middle`. Default left.",
            ),
            p(
                "count",
                ParamType::Int,
                false,
                "Click count: 2 for a double click. Default 1.",
            ),
            p(
                "mods",
                ParamType::StrList,
                false,
                "Modifiers held down: any of `ctrl`, `alt`, `shift`, `cmd`.",
            ),
        ],
    ),
    m(
        "mouse",
        "Send one raw mouse event, for drags and hovers a `click` cannot express.",
        &[
            p("action", ParamType::Str, true, "`down`, `up` or `move`."),
            p(
                "x",
                ParamType::Num,
                true,
                "Window-local logical pixels from the left edge.",
            ),
            p(
                "y",
                ParamType::Num,
                true,
                "Window-local logical pixels from the top edge.",
            ),
            p(
                "button",
                ParamType::Str,
                false,
                "`left`, `right` or `middle`. Default left.",
            ),
            p(
                "mods",
                ParamType::StrList,
                false,
                "Modifiers held down: any of `ctrl`, `alt`, `shift`, `cmd`.",
            ),
        ],
    ),
    m(
        "scroll",
        "Scroll the wheel over a point.",
        &[
            p(
                "x",
                ParamType::Num,
                true,
                "Window-local logical x of the pointer.",
            ),
            p(
                "y",
                ParamType::Num,
                true,
                "Window-local logical y of the pointer.",
            ),
            p(
                "dx",
                ParamType::Num,
                true,
                "Horizontal delta; positive scrolls content right.",
            ),
            p(
                "dy",
                ParamType::Num,
                true,
                "Vertical delta; negative scrolls down, as a wheel reports it.",
            ),
            p(
                "unit",
                ParamType::Str,
                false,
                "`lines` or `pixels`. Default lines.",
            ),
        ],
    ),
    m(
        "set_selection",
        "Move the editor cursor, or select a range, by byte offset.",
        &[
            p(
                "start",
                ParamType::Int,
                true,
                "Zero-based byte offset into the note's UTF-8 bytes.",
            ),
            p(
                "end",
                ParamType::Int,
                false,
                "Byte offset of the selection's other end. Defaults to `start`, i.e. no selection.",
            ),
        ],
    ),
    m(
        "scroll_to",
        "Scroll the editor so a line is in view.",
        &[p("line", ParamType::Int, true, "Zero-based line number.")],
    ),
    m(
        "focus",
        "Report what is focused, or move focus to a view.",
        &[p(
            "set",
            ParamType::Str,
            false,
            "View to focus: `workspace`, `editor`, `reading`, `graph`, `canvas` or `explorer` (the Files tree, shown first). Omit to only report.",
        )],
    ),
    m(
        "window",
        "Report the window's size and whether it is active, and optionally resize or raise it.",
        &[
            p(
                "activate",
                ParamType::Bool,
                false,
                "Ask the compositor to raise and focus the window, which a screenshot needs.",
            ),
            p(
                "width",
                ParamType::Num,
                false,
                "New width in logical pixels.",
            ),
            p(
                "height",
                ParamType::Num,
                false,
                "New height in logical pixels.",
            ),
        ],
    ),
    m(
        "read_file",
        "Read a vault file's bytes as text, exactly as they are on disk.",
        &[p(
            "path",
            ParamType::Str,
            true,
            "Vault-relative path; anything outside the vault is refused.",
        )],
    ),
    m(
        "write_file",
        "Write text to a vault file, creating it if it does not exist.",
        &[
            p(
                "path",
                ParamType::Str,
                true,
                "Vault-relative path; anything outside the vault is refused.",
            ),
            p(
                "text",
                ParamType::Str,
                true,
                "The whole new content. It is written verbatim, so keep the file's line endings \
                 and any BOM.",
            ),
        ],
    ),
    m(
        "list_files",
        "List the vault's files with their kind and size.",
        &[p(
            "folder",
            ParamType::Str,
            false,
            "Vault-relative folder to list, recursively. Omit for the whole vault.",
        )],
    ),
    m(
        "list_commands",
        "List every registered command with its name, hotkeys and whether it can run now.",
        NONE,
    ),
    m(
        "notice_clear",
        "Clear the status bar text, so the next command's notice is unambiguous.",
        NONE,
    ),
    m(
        "quit",
        "Quit the app. The reply is written before it exits.",
        NONE,
    ),
];

/// The spec for one method name, if it exists.
pub fn method(name: &str) -> Option<&'static MethodSpec> {
    METHODS.iter().find(|m| m.name == name)
}

/// JSON Schema (draft 2020-12 object schema) for one method's params.
pub fn params_schema(spec: &MethodSpec) -> Value {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    for param in spec.params {
        let mut schema = param.ty.schema();
        if let Some(obj) = schema.as_object_mut() {
            obj.insert("description".into(), Value::String(param.summary.into()));
        }
        properties.insert(param.name.to_string(), schema);
        if param.required {
            required.push(Value::String(param.name.to_string()));
        }
    }
    json!({
        "type": "object",
        "properties": Value::Object(properties),
        "required": Value::Array(required),
        "additionalProperties": false,
    })
}

/// The whole table as JSON, for the `protocol` method.
pub fn protocol_json() -> Value {
    let methods: Vec<Value> = METHODS
        .iter()
        .map(|spec| {
            let params: Vec<Value> = spec
                .params
                .iter()
                .map(|param| {
                    json!({
                        "name": param.name,
                        "type": param.ty.as_str(),
                        "required": param.required,
                        "summary": param.summary,
                    })
                })
                .collect();
            json!({
                "name": spec.name,
                "summary": spec.summary,
                "params": params,
                "params_schema": params_schema(spec),
            })
        })
        .collect();
    json!({
        "protocol": crate::PROTOCOL,
        "version": crate::VERSION,
        "methods": methods,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn method_names_are_unique() {
        let mut seen = HashSet::new();
        for spec in METHODS {
            assert!(seen.insert(spec.name), "duplicate method `{}`", spec.name);
            let mut params = HashSet::new();
            for param in spec.params {
                assert!(
                    params.insert(param.name),
                    "duplicate param `{}` on `{}`",
                    param.name,
                    spec.name
                );
            }
        }
    }

    #[test]
    fn every_method_documents_itself() {
        for spec in METHODS {
            assert!(!spec.name.is_empty());
            // A summary is help text, so it is a sentence, not a label.
            assert!(
                spec.summary.len() > 20 && spec.summary.ends_with('.'),
                "`{}` needs a real summary, got {:?}",
                spec.name,
                spec.summary
            );
            for param in spec.params {
                assert!(
                    param.summary.len() > 5,
                    "`{}`.`{}` needs a summary",
                    spec.name,
                    param.name
                );
            }
        }
    }

    #[test]
    fn methods_are_found_by_name() {
        assert_eq!(method("state").expect("exists").name, "state");
        assert!(method("frobnicate").is_none());
        // Required params come before optional ones everywhere, which is what
        // lets a client read a schema's `required` as a prefix of the list.
        for spec in METHODS {
            let mut optional_seen = false;
            for param in spec.params {
                if param.required {
                    assert!(
                        !optional_seen,
                        "`{}`: required `{}` follows an optional param",
                        spec.name, param.name
                    );
                } else {
                    optional_seen = true;
                }
            }
        }
    }

    #[test]
    fn a_schema_marks_required_params_required() {
        let spec = method("write_file").expect("exists");
        let schema = params_schema(spec);
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["properties"]["path"]["type"], "string");
        assert!(schema["properties"]["text"]["description"].is_string());
        let required: Vec<&str> = schema["required"]
            .as_array()
            .expect("an array")
            .iter()
            .map(|v| v.as_str().expect("a string"))
            .collect();
        assert_eq!(required, vec!["path", "text"]);

        // An optional param is a property but not required.
        let schema = params_schema(method("state").expect("exists"));
        assert_eq!(schema["properties"]["rows"]["type"], "integer");
        assert_eq!(schema["required"], serde_json::json!([]));

        // No params still schemas as an object, so every tool has one.
        let schema = params_schema(method("ping").expect("exists"));
        assert_eq!(schema["properties"], serde_json::json!({}));
        assert_eq!(schema["required"], serde_json::json!([]));

        // A list param keeps its item type.
        let schema = params_schema(method("keys").expect("exists"));
        assert_eq!(schema["properties"]["keys"]["type"], "array");
        assert_eq!(schema["properties"]["keys"]["items"]["type"], "string");
    }

    #[test]
    fn the_protocol_json_lists_every_method_in_order() {
        let json = protocol_json();
        assert_eq!(json["protocol"], crate::PROTOCOL);
        let methods = json["methods"].as_array().expect("an array");
        assert_eq!(methods.len(), METHODS.len());
        let names: Vec<&str> = methods
            .iter()
            .map(|m| m["name"].as_str().expect("a name"))
            .collect();
        let expected: Vec<&str> = METHODS.iter().map(|m| m.name).collect();
        assert_eq!(names, expected);
        assert_eq!(names[0], "ping");
        assert_eq!(*names.last().expect("non-empty"), "quit");
        assert_eq!(methods[0]["params_schema"]["type"], "object");
        let click = methods
            .iter()
            .find(|m| m["name"] == "click")
            .expect("click is a method");
        assert_eq!(click["params"][0]["name"], "x");
        assert_eq!(click["params"][0]["required"], true);
        assert_eq!(click["params"][4]["type"], "string[]");
    }
}
