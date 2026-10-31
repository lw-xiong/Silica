//! The control socket: the running app, driven from outside.
//!
//! This desktop has no input-automation tool, so a test or an MCP server has
//! no way to press a key in the window or click something in it. `--control
//! <path>` binds a Unix socket that speaks one line of JSON per request
//! (`silica-control` owns the wire types): it reports what the app is
//! showing, opens notes, runs commands by their Obsidian id, and feeds
//! keystrokes and mouse events into gpui's own dispatcher, which is as close
//! to a real user as the app can get from the inside.
//!
//! Every request is answered exactly once and nothing waits forever: KWin
//! stops delivering frame callbacks when the monitors sleep, so the waits
//! carry a deadline and report `timed_out` rather than leaving a client
//! blocked on a reply that can no longer come.

use crate::workspace::{context_names, ViewMode, Workspace};
use gpui::{
    point, prelude::*, px, size, App, AsyncApp, Keystroke, Modifiers, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, PlatformInput, Point, ScrollDelta, ScrollWheelEvent,
    TouchPhase, Window, WindowHandle,
};
use serde_json::{json, Value};
use silica_control::net::{UnixListener, UnixStream};
use silica_control::{Request, Response};
use std::cell::Cell;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

/// How often a deferred reply looks at the app again. Short enough that a
/// client's round trip is dominated by the thing it is waiting for.
const POLL: Duration = Duration::from_millis(8);

/// The longest any wait may be asked for. Nothing the socket waits for
/// takes ten minutes, and `timeout_ms` comes off the wire: an unbounded one
/// overflows the deadline rather than waiting for it.
const MAX_WAIT_MS: usize = 600_000;

/// The most frames a `wait_idle` may chain. Each one dirties the window, so
/// a caller's number is also a number of forced repaints.
const MAX_FRAMES: usize = 240;

/// The bound socket path, so both the `quit` method and the `Quit` action can
/// take the file away again.
static SOCKET: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Starts the control socket. One thread accepts connections and gives each
/// one a reader thread; the readers hand requests to the main thread and block
/// on the answer, so the app itself is only ever touched where gpui expects.
pub fn start(path: PathBuf, window: WindowHandle<Workspace>, cx: &mut App) {
    // A socket file outlives the run that bound it, and `bind` refuses an
    // existing path, so a stale one has to go. One that still answers is
    // not stale: taking its name away would leave another Silica's clients
    // talking to a listener nobody will read.
    if UnixStream::connect(&path).is_ok() {
        eprintln!(
            "control: {} is already serving a running Silica; not binding",
            path.display()
        );
        return;
    }
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("control: cannot bind {}: {error}", path.display());
            return;
        }
    };
    // Connecting to a Unix socket needs write permission on its name, and this
    // one opens and rewrites the vault: the default mode leaves it to the
    // umask, which on a stock login is permissive enough for anyone with a
    // local account. Windows has no mode to set: there the socket takes its
    // folder's ACL, so the caller chooses a folder of the user's own.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(error) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        {
            eprintln!("control: cannot restrict {}: {error}", path.display());
            let _ = std::fs::remove_file(&path);
            return;
        }
    }
    if let Ok(mut socket) = SOCKET.lock() {
        *socket = Some(path.clone());
    }
    eprintln!("control: listening on {}", path.display());

    let (requests, incoming) = async_channel::unbounded::<Pending>();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else {
                continue;
            };
            let requests = requests.clone();
            std::thread::spawn(move || serve(stream, requests));
        }
    });

    cx.spawn(async move |cx| {
        while let Ok(pending) = incoming.recv().await {
            let id = pending.request.id;
            let mut ctl = Ctl {
                win: window,
                cx: &mut *cx,
            };
            let line = match handle(&mut ctl, &pending.request, &pending.reply) {
                Ok(Some(result)) => Some(Response::ok(id, result).line()),
                Ok(None) => None,
                Err(error) => Some(Response::err(id, error).line()),
            };
            if let Some(line) = line {
                let _ = pending.reply.send(line);
            }
        }
    })
    .detach();
}

/// Removes the socket file, so the next run can bind the same path.
pub fn cleanup() {
    let Some(path) = SOCKET.lock().ok().and_then(|mut socket| socket.take()) else {
        return;
    };
    let _ = std::fs::remove_file(path);
}

/// One request waiting for the main thread, with the channel the connection
/// that sent it is blocked on.
struct Pending {
    request: Request,
    reply: mpsc::Sender<String>,
}

/// Reads request lines from one connection and blocks on the main thread's
/// answer, so a client sees exactly one response line per request, in order.
fn serve(stream: UnixStream, requests: async_channel::Sender<Pending>) {
    let Ok(reader) = stream.try_clone() else {
        return;
    };
    let mut writer = stream;
    for line in BufReader::new(reader).lines() {
        let Ok(line) = line else {
            return;
        };
        if line.trim().is_empty() {
            continue;
        }
        let request = match serde_json::from_str::<Request>(&line) {
            Ok(request) => request,
            Err(error) => {
                // A line that is not a request is still answered, under id 0,
                // and the connection stays open: a client that mistypes one
                // call carries on with the next.
                if write_line(&mut writer, &Response::err(0, error).line()).is_err() {
                    return;
                }
                continue;
            }
        };
        let id = request.id;
        // One channel per request: a handler that drops its sender without
        // answering fails the `recv` instead of parking this thread on a
        // reply that can never come.
        let (reply, replies) = mpsc::channel::<String>();
        if requests.send_blocking(Pending { request, reply }).is_err() {
            return;
        }
        let answer = match replies.recv() {
            Ok(answer) => answer,
            Err(_) => Response::err(id, "the app dropped the request").line(),
        };
        if write_line(&mut writer, &answer).is_err() {
            return;
        }
    }
}

fn write_line(stream: &mut UnixStream, line: &str) -> std::io::Result<()> {
    stream.write_all(line.as_bytes())?;
    stream.flush()
}

/// What a handler is allowed to reach: the window, and the workspace inside
/// it, one at a time.
struct Ctl<'a> {
    win: WindowHandle<Workspace>,
    cx: &'a mut AsyncApp,
}

impl Ctl<'_> {
    /// Runs `f` with the workspace, its window and its own context.
    fn workspace<R>(
        &mut self,
        f: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>) -> Result<R, String>,
    ) -> Result<R, String> {
        match self.win.update(self.cx, f) {
            Ok(result) => result,
            Err(error) => Err(error.to_string()),
        }
    }

    /// Runs `f` with the window and the app, leaving the workspace where it
    /// is. Input dispatch has to go through here: it re-enters whatever view
    /// is focused, and gpui panics on a circular lease if the workspace is
    /// already borrowed out of the entity map.
    fn window<R>(
        &mut self,
        f: impl FnOnce(&mut Window, &mut App) -> Result<R, String>,
    ) -> Result<R, String> {
        match self
            .cx
            .update_window(self.win.into(), |_, window, cx| f(window, cx))
        {
            Ok(result) => result,
            Err(error) => Err(error.to_string()),
        }
    }
}

/// Answers one request. `Ok(None)` means the reply is deferred: the handler
/// has taken a copy of `reply` and sends on it when the app is ready.
fn handle(
    ctl: &mut Ctl,
    request: &Request,
    reply: &mpsc::Sender<String>,
) -> Result<Option<Value>, String> {
    match request.method.as_str() {
        "ping" => Ok(Some(json!({
            "ok": true,
            "protocol": silica_control::PROTOCOL,
            "version": silica_control::VERSION,
        }))),
        "protocol" => Ok(Some(silica_control::protocol_json())),
        "wait_loaded" => {
            let timeout = request
                .opt_usize("timeout_ms")
                .unwrap_or(10_000)
                .min(MAX_WAIT_MS) as u64;
            wait_loaded(ctl, request.id, timeout, reply.clone());
            Ok(None)
        }
        "wait_idle" => {
            let frames = request.opt_usize("frames").unwrap_or(2).min(MAX_FRAMES);
            let timeout = request
                .opt_usize("timeout_ms")
                .unwrap_or(2_000)
                .min(MAX_WAIT_MS) as u64;
            wait_idle(ctl, request.id, frames, timeout, reply.clone())?;
            Ok(None)
        }
        "state" => {
            let text = request.bool_param("text", false);
            let rows = request.opt_usize("rows").unwrap_or(50);
            ctl.workspace(|ws, window, cx| Ok(ws.describe(text, rows, window, cx)))
                .map(Some)
        }
        "open" => {
            let name = request.str_param("name")?;
            let mode = request.opt_str("mode");
            ctl.workspace(|ws, window, cx| {
                if !ws.is_loaded() {
                    return Err("the vault is still loading".to_string());
                }
                // Validated first and applied after: "open X in mode M"
                // is about X, and switching before leaving would change the
                // mode of the note being left -- which its history entry
                // then remembers as the mode it was read in.
                let mode = ws.open_mode(mode)?;
                // A name that matches nothing is not an error: the
                // protocol answers with a null current file, so a caller
                // cannot mistake the note that is still open for the one it
                // asked for.
                if !ws.open_by_name(name, cx) {
                    return Ok(json!({"current": Value::Null}));
                }
                if let Some(mode) = mode {
                    ws.set_mode(mode, window, cx);
                }
                Ok(json!({"current": ws.current()}))
            })
            .map(Some)
        }
        "open_path" => {
            let path = request.str_param("path")?;
            let mode = request.opt_str("mode");
            ctl.workspace(|ws, window, cx| {
                // Every pane is keyed on the vault's own spelling of the
                // path; the caller's is only a request for it. Opening the
                // raw string instead would leave the explorer, the
                // backlinks and the outline looking up a key nothing has.
                let key = ws
                    .vault_file_path(path)
                    .ok_or_else(|| format!("no such file in the vault: {path}"))?;
                if let Some(mode) = ws.open_mode(mode)? {
                    ws.set_mode(mode, window, cx);
                }
                ws.open_file(key, None, cx);
                Ok(json!({"current": ws.current()}))
            })
            .map(Some)
        }
        "set_mode" => {
            let name = request.str_param("mode")?;
            let mode = ViewMode::parse(name).ok_or_else(|| format!("unknown mode: {name}"))?;
            ctl.workspace(|ws, window, cx| {
                ws.set_mode(mode, window, cx);
                Ok(json!({"mode": mode.as_str()}))
            })
            .map(Some)
        }
        "command" => {
            let id = request.str_param("id")?;
            ctl.workspace(|ws, window, cx| {
                ws.run_command(id, window, cx);
                Ok(json!({"status": ws.status_text()}))
            })
            .map(Some)
        }
        "keys" => {
            let names = request.str_list("keys")?;
            let strokes = names
                .iter()
                .map(|name| Keystroke::parse(name).map_err(|error| error.to_string()))
                .collect::<Result<Vec<_>, String>>()?;
            ctl.window(|window, cx| {
                let handled = strokes
                    .into_iter()
                    .map(|stroke| window.dispatch_keystroke(stroke, cx))
                    .collect::<Vec<bool>>();
                Ok(json!({"handled": handled, "pending": window.has_pending_keystrokes()}))
            })
            .map(Some)
        }
        "type" => {
            let text = request.str_param("text")?;
            match ctl.workspace(|ws, window, cx| Ok(ws.type_target(window, cx)))? {
                "editor" | "canvas-editor" => ctl
                    .workspace(|ws, window, cx| {
                        Ok(json!({"target": ws.type_into_focused(text, window, cx)}))
                    })
                    .map(Some),
                // Outside the workspace lease: one keystroke carrying the
                // whole string is what an IME commit looks like to gpui, and
                // delivering it re-enters whatever view is focused, which
                // may be the workspace itself.
                "input" => ctl
                    .window(|window, cx| {
                        let typed = Keystroke {
                            key_char: Some(text.to_string()),
                            ..Keystroke::default()
                        };
                        let handled = window.dispatch_keystroke(typed, cx);
                        Ok(json!({"target": if handled { "input" } else { "none" }}))
                    })
                    .map(Some),
                _ => Ok(Some(json!({"target": "none"}))),
            }
        }
        "click" => {
            let position = position(request)?;
            let button = button(request.opt_str("button"))?;
            let modifiers = modifiers(request)?;
            let click_count = request.opt_usize("count").unwrap_or(1);
            ctl.window(|window, cx| {
                // The move goes first so gpui's hover state and the hitboxes
                // an `on_click` was registered against agree on what is under
                // the pointer when the press lands.
                window.dispatch_event(
                    PlatformInput::MouseMove(MouseMoveEvent {
                        position,
                        pressed_button: None,
                        modifiers,
                    }),
                    cx,
                );
                window.dispatch_event(
                    PlatformInput::MouseDown(MouseDownEvent {
                        button,
                        position,
                        modifiers,
                        click_count,
                        first_mouse: false,
                    }),
                    cx,
                );
                window.dispatch_event(
                    PlatformInput::MouseUp(MouseUpEvent {
                        button,
                        position,
                        modifiers,
                        click_count,
                    }),
                    cx,
                );
                Ok(json!({"ok": true}))
            })
            .map(Some)
        }
        "mouse" => {
            let action = request.str_param("action")?;
            let position = position(request)?;
            let button = button(request.opt_str("button"))?;
            let modifiers = modifiers(request)?;
            let event = match action {
                // A move with a `button` named is a drag in progress: gpui
                // starts an element's drag only from a move that says which
                // button is held.
                "move" => PlatformInput::MouseMove(MouseMoveEvent {
                    position,
                    // Naming a button is what makes the move a drag; leaving
                    // it out is a hover, which is how a control that only
                    // appears under the pointer is revealed.
                    pressed_button: request.opt_str("button").map(|_| button),
                    modifiers,
                }),
                "down" => PlatformInput::MouseDown(MouseDownEvent {
                    button,
                    position,
                    modifiers,
                    click_count: 1,
                    first_mouse: false,
                }),
                "up" => PlatformInput::MouseUp(MouseUpEvent {
                    button,
                    position,
                    modifiers,
                    click_count: 1,
                }),
                other => return Err(format!("unknown mouse action: {other}")),
            };
            ctl.window(|window, cx| {
                window.dispatch_event(event, cx);
                Ok(json!({"ok": true}))
            })
            .map(Some)
        }
        "scroll" => {
            let position = position(request)?;
            let dx = request.f32_param("dx")?;
            let dy = request.f32_param("dy")?;
            let delta = match request.opt_str("unit").unwrap_or("lines") {
                "lines" => ScrollDelta::Lines(point(dx, dy)),
                "pixels" => ScrollDelta::Pixels(point(px(dx), px(dy))),
                other => return Err(format!("unknown scroll unit: {other}")),
            };
            let modifiers = modifiers(request)?;
            ctl.window(|window, cx| {
                window.dispatch_event(
                    PlatformInput::ScrollWheel(ScrollWheelEvent {
                        position,
                        delta,
                        modifiers,
                        touch_phase: TouchPhase::Moved,
                    }),
                    cx,
                );
                Ok(json!({"ok": true}))
            })
            .map(Some)
        }
        "set_selection" => {
            let start = request.usize_param("start")?;
            let end = request.opt_usize("end").unwrap_or(start);
            ctl.workspace(|ws, window, cx| {
                ws.editor().update(cx, |editor, cx| {
                    editor.set_selection_range(start, end, window, cx)
                });
                let dump = ws.editor().read(cx).describe(false, cx);
                let cursor = dump.get("cursor").cloned().unwrap_or(Value::Null);
                Ok(json!({"cursor": cursor}))
            })
            .map(Some)
        }
        "scroll_to" => {
            let line = request.usize_param("line")?;
            ctl.workspace(|ws, window, cx| {
                ws.editor()
                    .update(cx, |editor, cx| editor.scroll_to_line(line, window, cx));
                Ok(json!({"ok": true}))
            })
            .map(Some)
        }
        "focus" => {
            let set = request.opt_str("set");
            ctl.workspace(|ws, window, cx| {
                if let Some(name) = set {
                    ws.focus_view(name, window, cx)?;
                }
                Ok(json!({
                    "name": ws.focus_name(window, cx),
                    "context_stack": context_names(window),
                }))
            })
            .map(Some)
        }
        "window" => {
            let activate = request.bool_param("activate", false);
            let width = request.opt_f32("width");
            let height = request.opt_f32("height");
            let result = ctl.window(|window, _| {
                if activate {
                    window.activate_window();
                }
                if width.is_some() || height.is_some() {
                    let now = window.viewport_size();
                    let width = width.unwrap_or_else(|| f32::from(now.width));
                    let height = height.unwrap_or_else(|| f32::from(now.height));
                    // Zero, negative and non-finite are not sizes any
                    // compositor accepts, and the request carries whatever
                    // number the caller put in it.
                    if !(width.is_finite() && height.is_finite()) || width < 1.0 || height < 1.0 {
                        return Err(format!("{width}x{height} is not a window size"));
                    }
                    window.resize(size(px(width), px(height)));
                }
                // The window's origin is always zero on Wayland, so only the
                // size is worth reporting.
                let viewport = window.viewport_size();
                Ok(json!({
                    "width": f32::from(viewport.width),
                    "height": f32::from(viewport.height),
                    "scale": window.scale_factor(),
                    "active": window.is_window_active(),
                }))
            })?;
            // The title is the workspace's, not the platform window's: it
            // is what the last paint set, so it says which note and which
            // vault the window is showing.
            let title = ctl.workspace(|ws, _, _| Ok(ws.window_title()))?;
            let mut result = result;
            if let Value::Object(map) = &mut result {
                map.insert("title".into(), json!(title));
            }
            Ok(Some(result))
        }
        "read_file" => {
            // The path needs the vault, which only the main thread has; the
            // disk does not, and a large attachment or a slow mount would
            // hold up every frame and every other connection while it was
            // read.
            let path = request.str_param("path")?.to_string();
            let absolute = ctl.workspace(|ws, _, _| ws.vault_path(&path))?;
            let (id, reply) = (request.id, reply.clone());
            ctl.cx
                .spawn(async move |cx| {
                    let answer: Result<Value, String> = cx
                        .background_executor()
                        .spawn(async move {
                            let bytes = std::fs::read(&absolute)
                                .map_err(|error| format!("{path}: {error}"))?;
                            let count = bytes.len();
                            let text = String::from_utf8(bytes)
                                .map_err(|_| format!("{path} is not UTF-8"))?;
                            Ok(json!({"text": text, "bytes": count}))
                        })
                        .await;
                    let _ = reply.send(answer_line(id, answer));
                })
                .detach();
            Ok(None)
        }
        "write_file" => {
            let path = request.str_param("path")?.to_string();
            let text = request.str_param("text")?.to_string();
            let absolute = ctl.workspace(|ws, _, _| ws.vault_path(&path))?;
            let (id, reply) = (request.id, reply.clone());
            ctl.cx
                .spawn(async move |cx| {
                    let answer: Result<Value, String> = cx
                        .background_executor()
                        .spawn(async move {
                            let count = text.len();
                            // The caller's bytes land on disk exactly as
                            // given: nothing here preserves line endings or
                            // a BOM, because the caller supplies the whole
                            // file and Silica never rewrites bytes it did
                            // not change.
                            silica_vault::write_no_follow(&absolute, text.as_bytes())
                                .map_err(|error| format!("{path}: {error}"))?;
                            Ok(json!({"bytes": count}))
                        })
                        .await;
                    let _ = reply.send(answer_line(id, answer));
                })
                .detach();
            Ok(None)
        }
        "list_files" => {
            let folder = request.opt_str("folder");
            ctl.workspace(|ws, _, _| Ok(Value::Array(ws.list_vault_files(folder))))
                .map(Some)
        }
        "list_commands" => ctl
            .workspace(|ws, window, cx| Ok(Value::Array(ws.list_commands(window, cx))))
            .map(Some),
        "notice_clear" => ctl
            .workspace(|ws, _, cx| {
                ws.clear_status(cx);
                Ok(json!({"ok": true}))
            })
            .map(Some),
        "quit" => {
            ctl.workspace(|ws, _, cx| {
                ws.on_quit(cx);
                Ok(())
            })?;
            let _ = reply.send(Response::ok(request.id, json!({"ok": true})).line());
            cleanup();
            // The reply only reaches the client once the connection thread
            // wakes up and writes it, which cannot happen while this handler
            // is running: quit a beat later so the bytes get out.
            ctl.cx
                .spawn(async move |cx| {
                    cx.background_executor().timer(POLL * 6).await;
                    cx.update(|cx| cx.quit());
                })
                .detach();
            Ok(None)
        }
        other => Err(format!("unknown method: {other}")),
    }
}

/// One response line for an answer that came back from somewhere other
/// than the handler itself.
fn answer_line(id: u64, answer: Result<Value, String>) -> String {
    match answer {
        Ok(result) => Response::ok(id, result).line(),
        Err(error) => Response::err(id, error).line(),
    }
}

/// When a wait of `timeout_ms` runs out. The callers clamp the timeout
/// already, so the fallback is unreachable - but `Instant + Duration`
/// panics on overflow, and nothing that arrived over the socket may panic
/// the app.
fn wait_deadline(timeout_ms: u64) -> Instant {
    let now = Instant::now();
    now.checked_add(Duration::from_millis(timeout_ms.min(MAX_WAIT_MS as u64)))
        .unwrap_or(now)
}

/// Answers once the vault has finished indexing, or when the deadline passes.
fn wait_loaded(ctl: &mut Ctl, id: u64, timeout_ms: u64, reply: mpsc::Sender<String>) {
    let win = ctl.win;
    ctl.cx
        .spawn(async move |cx| {
            let deadline = wait_deadline(timeout_ms);
            loop {
                // "Loaded" has to mean "finished opening": the vault indexes
                // on a background thread and the startup flags only run once
                // it lands, so a client that answered to `is_loaded` alone
                // would read the window mid-arrangement and see no open note.
                let ready = win
                    .update(cx, |ws, _, _| ws.is_loaded() && ws.is_started())
                    .unwrap_or(false);
                let timed_out = !ready && Instant::now() >= deadline;
                if ready || timed_out {
                    let result = json!({"loaded": ready, "timed_out": timed_out});
                    let _ = reply.send(Response::ok(id, result).line());
                    return;
                }
                cx.background_executor().timer(POLL).await;
            }
        })
        .detach();
}

/// Answers once `frames` more frames have been rendered and the editor has
/// finished parsing, or when the deadline passes. The deadline is not
/// optional: a sleeping monitor stops the compositor's frame callbacks, and
/// then no number of frames is ever reached.
fn wait_idle(
    ctl: &mut Ctl,
    id: u64,
    frames: usize,
    timeout_ms: u64,
    reply: mpsc::Sender<String>,
) -> Result<(), String> {
    let drawn = Rc::new(Cell::new(0usize));
    let done = Rc::new(Cell::new(false));
    let (counter, stop) = (drawn.clone(), done.clone());
    ctl.window(move |window, _| {
        chain_frames(frames, counter, stop, window);
        Ok(())
    })?;
    let win = ctl.win;
    ctl.cx
        .spawn(async move |cx| {
            let deadline = wait_deadline(timeout_ms);
            loop {
                let parsing = win
                    .update(cx, |ws, _, cx| ws.editor().read(cx).is_parsing())
                    .unwrap_or(false);
                let idle = drawn.get() >= frames && !parsing;
                let timed_out = !idle && Instant::now() >= deadline;
                if idle || timed_out {
                    // Before the reply, and on the timed-out path too: the
                    // chain dirties the window at every step, so one that
                    // outlived its wait would repaint at full frame rate for
                    // the rest of the run.
                    done.set(true);
                    let result =
                        json!({"frames": drawn.get(), "parsing": parsing, "timed_out": timed_out});
                    let _ = reply.send(Response::ok(id, result).line());
                    return;
                }
                cx.background_executor().timer(POLL).await;
            }
        })
        .detach();
    Ok(())
}

/// Counts the next `remaining` rendered frames, until the wait that asked
/// for them sets `done`. Each step dirties the window first: a window with
/// nothing to redraw never asks for a frame, and then the callback would
/// never run.
fn chain_frames(
    remaining: usize,
    drawn: Rc<Cell<usize>>,
    done: Rc<Cell<bool>>,
    window: &mut Window,
) {
    if remaining == 0 || done.get() {
        return;
    }
    window.refresh();
    window.on_next_frame(move |window, _| {
        drawn.set(drawn.get() + 1);
        chain_frames(remaining - 1, drawn, done, window);
    });
}

/// Window-local logical pixels, the only coordinates the protocol speaks: on
/// Wayland there is no window origin to translate screen ones through.
fn position(request: &Request) -> Result<Point<Pixels>, String> {
    Ok(point(
        px(request.f32_param("x")?),
        px(request.f32_param("y")?),
    ))
}

fn button(name: Option<&str>) -> Result<MouseButton, String> {
    match name.unwrap_or("left") {
        "left" => Ok(MouseButton::Left),
        "right" => Ok(MouseButton::Right),
        "middle" => Ok(MouseButton::Middle),
        other => Err(format!("unknown mouse button: {other}")),
    }
}

fn modifiers(request: &Request) -> Result<Modifiers, String> {
    let mut modifiers = Modifiers::none();
    // Absent and `null` mean the same thing here as everywhere else in the
    // protocol: no modifiers held.
    if request.params.get("mods").is_none_or(Value::is_null) {
        return Ok(modifiers);
    }
    for name in request.str_list("mods")? {
        match name.as_str() {
            "ctrl" | "control" => modifiers.control = true,
            "alt" | "option" => modifiers.alt = true,
            "shift" => modifiers.shift = true,
            "cmd" | "super" | "platform" => modifiers.platform = true,
            other => return Err(format!("unknown modifier: {other}")),
        }
    }
    Ok(modifiers)
}
