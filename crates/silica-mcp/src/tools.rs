//! The tool table: what a model may call, and what each call does.
//!
//! Descriptions are the only documentation a model gets, so each one says when
//! to reach for the tool and what comes back. The schemas of the thin wrappers
//! are generated from [`silica_control::METHODS`] rather than written out
//! again: the socket's parameters and the tool's parameters are then the same
//! list by construction, and renaming a control parameter cannot leave a model
//! calling the old name.

use crate::app::App;
use crate::base64;
use crate::rpc::{Fault, INVALID_PARAMS};
use serde_json::{json, Map, Value};
use silica_control::{method, params_schema};

/// How a tool runs.
enum Run {
    /// A thin wrapper over one control method. `settles` asks for an automatic
    /// `wait_idle` afterwards, so the next state dump or screenshot describes
    /// the window as it is now rather than mid-edit.
    Method {
        method: &'static str,
        settles: bool,
    },
    Launch,
    Quit,
    Screenshot,
    /// `list_commands` plus a client-side filter; the socket has no filter.
    ListCommands,
}

struct Tool {
    name: &'static str,
    description: &'static str,
    run: Run,
}

/// Shorthand for the wrappers, which are most of the table.
const fn wrap(name: &'static str, description: &'static str, method: &'static str) -> Tool {
    Tool {
        name,
        description,
        run: Run::Method {
            method,
            settles: false,
        },
    }
}

/// A wrapper whose method changes the window, so it waits for the app to
/// settle before answering.
const fn act(name: &'static str, description: &'static str, method: &'static str) -> Tool {
    Tool {
        name,
        description,
        run: Run::Method {
            method,
            settles: true,
        },
    }
}

const TOOLS: &[Tool] = &[
    Tool {
        name: "silica_launch",
        description: "\
Start Silica on a vault and wait until it has finished indexing. Call this \
first: every other tool fails while no app is running. `vault` is an absolute \
path to the vault folder (the one holding `.obsidian`). `note` opens a note at \
startup (`Name`, `Name#Heading` or `Name#^blockid`), `mode` is `reading`, \
`live`, `source` or `graph`, and `width`/`height` (window-local logical \
pixels, both or neither) fix the window size, which is worth doing when you \
intend to compare screenshots. Returns a short summary — vault, open file, \
mode, focus — not the whole state dump; use silica_state for that. Fails if \
this server already launched an app: call silica_quit first.",
        run: Run::Launch,
    },
    Tool {
        name: "silica_quit",
        description: "\
Ask the running app to exit, then reap it and remove the control socket. Use \
it when you are done driving the app, or to get back to a clean window after \
leaving it in a strange state. Always quit explicitly: the app is only taken \
down with this server when the server shuts down on stdin closing, and an MCP \
client that stops it with a signal instead leaves the window, the vault and \
the socket behind for somebody to kill by hand.",
        run: Run::Quit,
    },
    wrap(
        "silica_state",
        "\
Dump everything the app can see, as JSON: the vault, the open file and its \
history, both docks (file explorer, search results, backlinks, outgoing links, \
tags, all properties, outline), any open modal, the editor (cursor, selection, scroll, visible \
lines, find panel, autocompletion popup), the reading view, the graph, the \
status bar text, what has focus, and the window size. Read this before and \
after anything else — it is how you find out what actually happened, and it is \
exact where a screenshot is a guess. `text: true` adds the open note's full \
text; `rows` (default 50) caps every list, and a capped list gets a `*_total` \
sibling with the real count.",
        "state",
    ),
    Tool {
        name: "silica_screenshot",
        description: "\
Activate the window, let it settle, and return a PNG of it. Use it when the \
question is visual — layout, colors, what a popup looks like, whether \
something overlaps. For anything readable as data, silica_state is cheaper and \
precise. The capture command comes from `SILICA_MCP_SCREENSHOT` (default \
`spectacle -b -n -a -o {path}`; on Windows, a PowerShell capture of the \
foreground window). `crop` is an ImageMagick geometry such as \
`800x600+0+0`, and works only if that template has a `{crop}` placeholder for \
it, since this server does not decode the image itself.",
        run: Run::Screenshot,
    },
    act(
        "silica_open",
        "\
Open a note by name, the way the quick switcher does: `Name`, `Name#Heading` \
or `Name#^blockid`, resolved against the vault with Obsidian's rules \
(case-insensitive, shortest unique path). `mode` is `reading`, `live`, \
`source` or `graph`. Returns the path that is now current, or null when the \
name resolved to nothing.",
        "open",
    ),
    act(
        "silica_set_mode",
        "\
Switch the active view: `reading` (rendered), `live` (Live Preview), `source` \
(raw Markdown) or `graph`.",
        "set_mode",
    ),
    act(
        "silica_command",
        "\
Run a command by its Obsidian command id, such as `editor:toggle-bold` or \
`workspace:split-vertical` — the same ids the command palette and the hotkey \
table use. Running a command is the reliable way to do something the UI can \
do; prefer it over hunting for the button. Returns the status bar text left \
behind, which is often the only report a command makes. Use \
silica_list_commands to find an id and check it can run right now.",
        "command",
    ),
    act(
        "silica_keys",
        "\
Send keystrokes to whatever has focus, spelled the way gpui spells them: \
`[\"ctrl-shift-f\", \"enter\"]`, modifiers `ctrl`, `alt`, `shift`, `cmd`, \
joined to the key with `-`. This is how you drive modals, hotkeys and \
navigation. Returns whether each keystroke was handled and whether a multi-key \
binding is still pending. For literal text use silica_type instead — it does \
not depend on a keymap.",
        "keys",
    ),
    act(
        "silica_type",
        "\
Type literal text wherever the focus is: the editor, a modal's input, or a \
property or table field. Returns which of those received it — `editor`, \
`input`, or `none`, and `none` means nothing was focused, so check \
silica_state's `focus` or move focus with silica_focus first.",
        "type",
    ),
    act(
        "silica_click",
        "\
Click in the window. `x` and `y` are window-local logical pixels: 0,0 is the \
top-left of the window's content, they are not screen coordinates and not \
physical pixels, so they do not change with the display scale. To find a \
coordinate, read silica_state (it reports the window size in the same units, \
plus the rows of each pane and what each row is) and take a silica_screenshot \
to see where things sit; between them you can aim. `button` is `left`, `right` \
or `middle`, `count` is 2 for a double click, and `mods` is an array of \
`ctrl`, `alt`, `shift`, `cmd` — ctrl-click is how Obsidian opens a link in a \
new pane.",
        "click",
    ),
    act(
        "silica_mouse",
        "\
Press, move or release the mouse, for the gestures silica_click cannot make: \
a drag. `action` is `down`, `move` or `up`, with `x` and `y` in window-local \
logical pixels as for silica_click, and `button` is `left`, `right` or \
`middle`. A `move` that names a `button` is a drag in progress -- that is what \
tells the app a button is held, and without it a move is only a hover. So a \
drag-select is `down`, then one or more `move`s naming the same button, then \
`up`: read `editor.selection` in silica_state afterwards to see what it \
caught. A `move` with no `button` is how you hover something, which is the \
only way to reveal a control that appears on hover, such as a code block's \
copy button.",
        "mouse",
    ),
    act(
        "silica_window",
        "\
Read the window's size, scale, title and whether it is active, and optionally \
resize it (`width`, `height`) or raise it (`activate`). Activate before \
anything that depends on the window really having focus: a Wayland client \
cannot write the clipboard without it, so a copy driven while another \
application holds focus silently does nothing.",
        "window",
    ),
    act(
        "silica_scroll",
        "\
Scroll whatever is under `x`,`y` (window-local logical pixels, as for \
silica_click) by `dx`,`dy`. `unit` is `lines` (default, one wheel notch per \
line) or `pixels`. Positive `dy` moves the content the way a wheel pulled \
towards you does. Read back `editor.scroll` or `reading.scroll` in \
silica_state to see where you landed.",
        "scroll",
    ),
    act(
        "silica_select",
        "\
Select a byte range in the open editor. `start` and `end` are zero-based byte \
offsets into the note's UTF-8 bytes, like Obsidian's Pos — get them from \
silica_state's `editor.cursor.offset`, or from the outline and backlink \
entries, which carry offsets. `end` defaults to `start`, which just moves the \
cursor. Returns the resulting cursor line, column and offset.",
        "set_selection",
    ),
    wrap(
        "silica_wait_idle",
        "\
Block until the app has rendered `frames` quiet frames (default 2) and the \
editor has stopped parsing, or `timeout_ms` (default 2000) elapses. Every tool \
that changes the window already does this, so you need it only after something \
slow of your own — a command that re-indexes the vault, say. A `timed_out: \
true` answer is a warning, not a failure: on a sleeping display the compositor \
stops delivering frames, and the timeout is what keeps this from hanging.",
        "wait_idle",
    ),
    wrap(
        "silica_read_file",
        "\
Read a vault file as text. `path` is vault-relative, `/`-separated, original \
case; a path that escapes the vault is refused. Returns the text and its byte \
length. This reads the file on disk, so unsaved editor changes are not in it — \
`silica_state` with `text: true` is what the editor holds.",
        "read_file",
    ),
    wrap(
        "silica_write_file",
        "\
Write a vault file, replacing whatever was there. `path` is vault-relative; a \
path that escapes the vault is refused. The vault folder is the user's source \
of truth, so prefer changing a note through the app (silica_type, silica_keys, \
silica_command), which goes through the editor and preserves what the app \
knows about the file; use this for setup you mean to persist, such as writing \
a fixture note before driving the UI over it. Returns the bytes written.",
        "write_file",
    ),
    wrap(
        "silica_list_files",
        "\
List the vault's files with their vault-relative path, kind (markdown, canvas, \
image, pdf, other) and size in bytes. `folder` lists one vault-relative folder \
instead of the whole vault. Use it to find a note's exact path before \
silica_open or silica_read_file.",
        "list_files",
    ),
    Tool {
        name: "silica_list_commands",
        description: "\
List the commands the app knows: Obsidian command id, display name, bound \
hotkeys, and whether the command can run right now (many need an open note). \
`filter` keeps only commands whose id or name contains it, case-insensitively \
— the list is long, so filter it. This is where you find the id silica_command \
needs, and the hotkey silica_keys would press.",
        run: Run::ListCommands,
    },
    wrap(
        "silica_focus",
        "\
Report what has keyboard focus and the gpui key context stack above it, which \
is what decides whether a hotkey fires at all — a key that does nothing is \
usually a focus problem, and this is how you see it. `set` moves focus to \
`workspace`, `editor`, `reading`, `graph`, `canvas`, `bases` or `explorer` \
(the Files tree).",
        "focus",
    ),
];

/// Every tool, with the schema a client validates calls against.
pub fn list() -> Value {
    let tools: Vec<Value> = TOOLS
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchema": schema(tool),
            })
        })
        .collect();
    json!({ "tools": tools })
}

/// Run one `tools/call`. The `Err` arm is for a malformed request only: a tool
/// that ran and failed is an `isError` result, which the model can read.
pub fn call(app: &mut App, params: &Value) -> Result<Value, Fault> {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return Err(Fault::new(INVALID_PARAMS, "tools/call needs a `name`"));
    };
    let Some(tool) = TOOLS.iter().find(|tool| tool.name == name) else {
        return Err(Fault::new(INVALID_PARAMS, format!("unknown tool: {name}")));
    };
    let arguments = match params.get("arguments") {
        None | Some(Value::Null) => Value::Object(Map::new()),
        Some(arguments) if arguments.is_object() => arguments.clone(),
        Some(_) => return Err(Fault::new(INVALID_PARAMS, "`arguments` must be an object")),
    };
    Ok(match run(app, tool, &arguments) {
        Ok(content) => json!({"content": content, "isError": false}),
        Err(message) => json!({"content": [text(message)], "isError": true}),
    })
}

fn run(app: &mut App, tool: &Tool, arguments: &Value) -> Result<Vec<Value>, String> {
    match tool.run {
        Run::Launch => Ok(vec![text(app.launch(arguments)?)]),
        Run::Quit => Ok(vec![text(app.quit()?)]),
        Run::Screenshot => {
            let png = app.screenshot(str_arg(arguments, "crop"))?;
            Ok(vec![
                text(format!("{} bytes of PNG", png.len())),
                json!({"type": "image", "data": base64::encode(&png), "mimeType": "image/png"}),
            ])
        }
        Run::ListCommands => {
            let filter = str_arg(arguments, "filter")
                .unwrap_or_default()
                .to_lowercase();
            let commands = app.call("list_commands", json!({}))?;
            let all = commands
                .as_array()
                .ok_or_else(|| format!("list_commands answered {commands}, not an array"))?;
            let kept: Vec<Value> = all
                .iter()
                .filter(|command| matches(command, &filter))
                .cloned()
                .collect();
            Ok(vec![text(format!(
                "{} of {} commands{}\n{}",
                kept.len(),
                all.len(),
                if filter.is_empty() {
                    String::new()
                } else {
                    format!(" matching {filter:?}")
                },
                pretty(&Value::Array(kept)),
            ))])
        }
        Run::Method { method, settles } => {
            let result = app.call(method, arguments.clone())?;
            let mut content = vec![text(pretty(&result))];
            // The call succeeded; a timed-out settle is worth saying but is not
            // a failure of the tool the model asked for.
            if settles {
                if let Err(e) = app.call("wait_idle", json!({})) {
                    content.push(text(format!("(wait_idle after `{method}` failed: {e})")));
                }
            }
            Ok(content)
        }
    }
}

/// The input schema. Only the four tools that are not a thin wrapper describe
/// their own parameters; the rest come from the protocol table.
fn schema(tool: &Tool) -> Value {
    match tool.run {
        Run::Launch => json!({
            "type": "object",
            "properties": {
                "vault": {"type": "string",
                    "description": "Absolute path to the vault folder."},
                "note": {"type": "string",
                    "description": "Note to open at startup: `Name`, `Name#Heading` or `Name#^blockid`."},
                "mode": {"type": "string", "enum": ["reading", "live", "source", "graph"],
                    "description": "View to open the note in; the vault's default when omitted."},
                "width": {"type": "number",
                    "description": "Window width in logical pixels; give height too."},
                "height": {"type": "number",
                    "description": "Window height in logical pixels; give width too."},
            },
            "required": ["vault"],
        }),
        Run::Quit => json!({"type": "object", "properties": {}}),
        Run::Screenshot => json!({
            "type": "object",
            "properties": {
                "crop": {"type": "string",
                    "description": "ImageMagick geometry to crop to, e.g. `800x600+0+0`; needs a `{crop}` placeholder in SILICA_MCP_SCREENSHOT."},
            },
        }),
        Run::ListCommands => json!({
            "type": "object",
            "properties": {
                "filter": {"type": "string",
                    "description": "Keep only commands whose id or name contains this, case-insensitively."},
            },
        }),
        Run::Method { method: name, .. } => match method(name) {
            Some(spec) => params_schema(spec),
            // Unreachable while the table above names real methods, and a
            // permissive schema beats a tool that cannot be listed.
            None => json!({"type": "object"}),
        },
    }
}

/// Does one `list_commands` row match the filter?
fn matches(command: &Value, filter: &str) -> bool {
    if filter.is_empty() {
        return true;
    }
    ["id", "name"].iter().any(|key| {
        command
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|value| value.to_lowercase().contains(filter))
    })
}

fn str_arg<'a>(arguments: &'a Value, name: &str) -> Option<&'a str> {
    arguments.get(name).and_then(Value::as_str)
}

fn text(body: impl Into<String>) -> Value {
    json!({"type": "text", "text": body.into()})
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Config;
    use crate::rpc;
    use silica_control::net::UnixListener;
    use std::io::{BufRead, BufReader, Write};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    /// A stand-in for the app: it answers canned results on a real Unix socket
    /// and records which control methods were asked for, so every tool can be
    /// driven with no window, no GPU and no vault.
    struct Fake {
        path: PathBuf,
        asked: Arc<Mutex<Vec<String>>>,
    }

    impl Fake {
        /// `answers` pairs a control method with the result it returns;
        /// anything else gets an error, the way the app's socket would.
        fn start(name: &str, answers: Vec<(&'static str, Value)>) -> Fake {
            // `tempfile` is not a dependency here, so the pid and the test
            // name keep two runs apart.
            let path =
                std::env::temp_dir().join(format!("silica-mcp-{}-{name}.sock", std::process::id()));
            let _ = std::fs::remove_file(&path);
            let listener = UnixListener::bind(&path).expect("bind the fake socket");
            let asked = Arc::new(Mutex::new(Vec::new()));
            let recorded = Arc::clone(&asked);
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let reader = BufReader::new(match stream.try_clone() {
                        Ok(stream) => stream,
                        Err(_) => break,
                    });
                    let mut writer = stream;
                    for line in reader.lines().map_while(Result::ok) {
                        let Ok(request) = serde_json::from_str::<Value>(&line) else {
                            continue;
                        };
                        let method = request["method"].as_str().unwrap_or_default().to_string();
                        let id = request["id"].clone();
                        let reply = match answers.iter().find(|(name, _)| *name == method) {
                            Some((_, result)) => json!({"id": id, "result": result}),
                            None => json!({"id": id, "error": format!("unknown method: {method}")}),
                        };
                        recorded.lock().expect("record the method").push(method);
                        if writeln!(writer, "{reply}").is_err() {
                            break;
                        }
                    }
                }
            });
            Fake { path, asked }
        }

        fn app(&self) -> App {
            App::new(Config::for_test(self.path.clone()))
        }

        fn asked(&self) -> Vec<String> {
            self.asked.lock().expect("read the methods").clone()
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    /// `tools/call` as a client sends it.
    fn called(app: &mut App, name: &str, arguments: Value) -> Value {
        call(app, &json!({"name": name, "arguments": arguments})).expect("not a protocol fault")
    }

    fn body(result: &Value) -> &str {
        result["content"][0]["text"].as_str().expect("text content")
    }

    #[test]
    fn every_tool_is_listed_with_a_schema() {
        let listed = list();
        let tools = listed["tools"].as_array().expect("an array of tools");
        assert_eq!(tools.len(), TOOLS.len());
        let mut names: Vec<&str> = tools
            .iter()
            .map(|tool| tool["name"].as_str().expect("a name"))
            .collect();
        for tool in tools {
            let name = tool["name"].as_str().expect("a name");
            assert!(name.starts_with("silica_"), "{name}");
            let description = tool["description"].as_str().expect("a description");
            assert!(description.len() > 40, "{name} is barely documented");
            assert_eq!(tool["inputSchema"]["type"], "object", "{name}");
        }
        names.sort_unstable();
        let unique = names.len();
        names.dedup();
        assert_eq!(names.len(), unique, "two tools share a name");
    }

    #[test]
    fn wrapper_schemas_come_from_the_protocol_table() {
        let listed = list();
        let tools = listed["tools"].as_array().expect("tools");
        let find = |name: &str| {
            tools
                .iter()
                .find(|tool| tool["name"] == name)
                .expect("the tool")
                .clone()
        };
        let click = find("silica_click");
        let properties = &click["inputSchema"]["properties"];
        for name in ["x", "y", "button", "count", "mods"] {
            assert!(!properties[name].is_null(), "silica_click lost {name}");
        }
        let required = click["inputSchema"]["required"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert!(required.contains(&json!("x")), "{required:?}");
        // The hand-written four are the only ones that may invent a parameter.
        let launch = find("silica_launch");
        assert_eq!(launch["inputSchema"]["required"], json!(["vault"]));
        assert!(!find("silica_list_commands")["inputSchema"]["properties"]["filter"].is_null());
    }

    #[test]
    fn a_wrapper_passes_its_arguments_through_and_settles() {
        let fake = Fake::start(
            "open",
            vec![
                ("open", json!({"current": "Sub/Alpha.md"})),
                (
                    "wait_idle",
                    json!({"frames": 2, "parsing": false, "timed_out": false}),
                ),
            ],
        );
        let mut app = fake.app();
        let result = called(
            &mut app,
            "silica_open",
            json!({"name": "Alpha", "mode": "source"}),
        );
        assert_eq!(result["isError"], json!(false), "{result}");
        assert!(body(&result).contains("Sub/Alpha.md"), "{result}");
        // A state-changing tool waits for the window before answering.
        assert_eq!(fake.asked(), vec!["open", "wait_idle"]);
    }

    #[test]
    fn a_read_only_wrapper_does_not_wait() {
        let fake = Fake::start(
            "state",
            vec![("state", json!({"vault": {"name": "PLMCRZY"}}))],
        );
        let mut app = fake.app();
        let result = called(&mut app, "silica_state", json!({"rows": 5}));
        assert_eq!(result["isError"], json!(false));
        // Pretty-printed, because a model reads it.
        assert!(body(&result).contains("\n  \"vault\""), "{}", body(&result));
        assert_eq!(fake.asked(), vec!["state"]);
    }

    #[test]
    fn a_method_error_is_an_is_error_result() {
        let fake = Fake::start("badcommand", vec![("wait_idle", json!({}))]);
        let mut app = fake.app();
        let result = called(&mut app, "silica_command", json!({"id": "nope:nope"}));
        assert_eq!(result["isError"], json!(true), "{result}");
        assert!(
            body(&result).contains("unknown method: command"),
            "{result}"
        );
        // A method that failed changed nothing, so there is nothing to wait for.
        assert_eq!(fake.asked(), vec!["command"]);
    }

    #[test]
    fn list_commands_filters_on_id_and_name() {
        let fake = Fake::start(
            "commands",
            vec![(
                "list_commands",
                json!([
                    {"id": "editor:toggle-bold", "name": "Toggle bold", "hotkeys": ["ctrl-b"], "runnable": true},
                    {"id": "app:go-back", "name": "Navigate back", "hotkeys": [], "runnable": true},
                    {"id": "workspace:split-vertical", "name": "Split right", "hotkeys": [], "runnable": false},
                ]),
            )],
        );
        let mut app = fake.app();
        let all = called(&mut app, "silica_list_commands", json!({}));
        assert!(
            body(&all).starts_with("3 of 3 commands\n"),
            "{}",
            body(&all)
        );

        let bold = called(&mut app, "silica_list_commands", json!({"filter": "BOLD"}));
        let text = body(&bold);
        assert!(
            text.starts_with("1 of 3 commands matching \"bold\""),
            "{text}"
        );
        assert!(text.contains("editor:toggle-bold"), "{text}");
        assert!(!text.contains("app:go-back"), "{text}");

        // The filter reads the name as well as the id.
        let back = called(
            &mut app,
            "silica_list_commands",
            json!({"filter": "navigate"}),
        );
        assert!(body(&back).contains("app:go-back"), "{}", body(&back));
    }

    #[test]
    fn a_screenshot_reports_what_the_capture_command_did() {
        let fake = Fake::start(
            "shot",
            vec![
                ("window", json!({"active": true})),
                ("wait_idle", json!({})),
            ],
        );
        let mut app = fake.app();
        // `Config::for_test` captures with `false`, which writes nothing.
        let result = called(&mut app, "silica_screenshot", json!({}));
        assert_eq!(result["isError"], json!(true), "{result}");
        assert!(
            body(&result).contains("screenshot command failed"),
            "{result}"
        );
        assert_eq!(fake.asked(), vec!["window", "wait_idle"]);
    }

    #[test]
    fn a_crop_the_template_cannot_honour_fails_before_the_window_is_touched() {
        let fake = Fake::start("crop", vec![("window", json!({}))]);
        let mut app = fake.app();
        let result = called(
            &mut app,
            "silica_screenshot",
            json!({"crop": "800x600+0+0"}),
        );
        assert_eq!(result["isError"], json!(true));
        assert!(body(&result).contains("SILICA_MCP_SCREENSHOT"), "{result}");
        assert!(fake.asked().is_empty(), "the window was poked anyway");
    }

    #[test]
    fn launch_refuses_a_vault_that_is_not_there() {
        let fake = Fake::start("launch", vec![]);
        let mut app = fake.app();
        let missing = std::env::temp_dir().join("silica-mcp-no-such-vault");
        let result = called(&mut app, "silica_launch", json!({"vault": missing}));
        assert_eq!(result["isError"], json!(true), "{result}");
        assert!(
            body(&result).contains("silica-mcp-no-such-vault"),
            "{result}"
        );

        let result = called(&mut app, "silica_launch", json!({}));
        assert!(body(&result).contains("`vault`"), "{result}");
    }

    #[test]
    fn launch_refuses_a_binary_that_is_not_there() {
        let fake = Fake::start("nobin", vec![]);
        let mut app = fake.app();
        // A folder that certainly exists, so the binary is what fails.
        let vault = std::env::temp_dir();
        let result = called(
            &mut app,
            "silica_launch",
            json!({"vault": vault, "width": 1400}),
        );
        assert_eq!(result["isError"], json!(true), "{result}");
        assert!(body(&result).contains("width"), "{result}");

        let result = called(&mut app, "silica_launch", json!({"vault": vault}));
        assert!(body(&result).contains("/nonexistent/silica"), "{result}");
        assert!(body(&result).contains("SILICA_MCP_BIN"), "{result}");
    }

    #[test]
    fn quit_asks_the_app_and_cleans_up_after_it() {
        let fake = Fake::start("quit", vec![("quit", json!({"ok": true}))]);
        let mut app = fake.app();
        // The fake socket is listening, so `quit` is attempted and answered;
        // with no child of our own there is nothing to reap.
        let result = called(&mut app, "silica_quit", json!({}));
        assert_eq!(result["isError"], json!(false), "{result}");
        assert_eq!(fake.asked(), vec!["quit"]);
    }

    #[test]
    fn a_call_arrives_through_the_jsonrpc_loop() {
        let fake = Fake::start(
            "loop",
            vec![
                (
                    "focus",
                    json!({"name": "editor", "context_stack": ["Workspace", "Editor"]}),
                ),
                ("wait_idle", json!({})),
            ],
        );
        let mut app = fake.app();
        let line = r#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"silica_focus","arguments":{"set":"editor"}}}"#;
        let reply: Value =
            serde_json::from_str(&rpc::handle_line(&mut app, line).expect("a reply"))
                .expect("JSON");
        assert_eq!(reply["id"], json!(11));
        assert_eq!(reply["result"]["isError"], json!(false), "{reply}");
        assert!(reply["result"]["content"][0]["text"]
            .as_str()
            .expect("text")
            .contains("Workspace"));
    }

    #[test]
    fn bad_tool_calls_are_protocol_faults() {
        let fake = Fake::start("faults", vec![]);
        let mut app = fake.app();
        let fault = call(&mut app, &json!({})).expect_err("no name");
        assert_eq!(fault.code, INVALID_PARAMS);
        let fault = call(&mut app, &json!({"name": "silica_dance"})).expect_err("unknown tool");
        assert_eq!(fault.code, INVALID_PARAMS);
        let fault = call(&mut app, &json!({"name": "silica_state", "arguments": 7}))
            .expect_err("arguments must be an object");
        assert_eq!(fault.code, INVALID_PARAMS);
        // Absent arguments are an empty object, not a fault.
        assert!(call(&mut app, &json!({"name": "silica_state"})).is_ok());
    }
}
