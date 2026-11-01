//! Silica application.
//!
//! `silica <vault> [--note <path>]` opens a vault in the three-pane shell:
//! Files on the left, the reading view (or the source editor, Ctrl+E) in the
//! middle, and Backlinks, Outline and Tags on the right. `--synthetic N`
//! and `--stress` keep the editor spike from the first gate runnable; the
//! other flags open the app in a given state for screenshots and smoke
//! tests, and `--control <socket>` lets a test or the MCP server drive the
//! running app instead (see `docs/silica/Development.md`).

mod assets;
mod backlinks;
mod bases;
mod bookmarks;
mod canvas;
mod chooser;
mod control;
mod copy_results;
mod editor;
mod explorer;
mod fold;
mod graph;
mod help;
mod http_client;
mod math;
mod mentions;
mod mermaid;
mod palette;
mod panes;
mod pdf;
mod preview;
mod page_preview;
mod prompt;
mod properties;
mod reading;
mod search;
mod settings;
mod suggest;
mod switcher;
mod text_input;
mod theme;
mod title_field;
mod vaults;
mod watch;
mod workspace;

use gpui::{
    actions, point, prelude::*, px, size, App, Bounds, Focusable, KeyBinding, TitlebarOptions,
    WindowAppearance, WindowBackgroundAppearance, WindowBounds, WindowDecorations, WindowOptions,
};
use silica_commands::CommandRegistry;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use theme::Theme;
use workspace::{Commands, ViewMode, Workspace};

actions!(silica, [Quit]);

/// A deterministic note of `lines` lines exercising headings, wikilinks,
/// tags, emphasis, code, lists, and callouts, for the 50k-line spike.
fn synthetic_note(lines: usize) -> String {
    const WORDS: &[&str] = &[
        "vault", "note", "link", "graph", "heading", "callout", "tag", "cursor", "scroll",
        "render", "parser", "cache", "index", "editor", "canvas", "search", "theme", "plugin",
    ];
    let mut out = String::with_capacity(lines * 48);
    let mut seed: u64 = 7;
    let mut pick = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        WORDS[(seed % WORDS.len() as u64) as usize]
    };
    out.push_str("---\ntitle: Synthetic\ntags: [spike]\n---\n");
    let mut n = 4;
    while n < lines {
        match n % 40 {
            0 => {
                let _ = writeln!(out, "\n## Section {} {}", n / 40, pick());
                n += 2;
            }
            7 => {
                let _ = writeln!(
                    out,
                    "> [!tip] {} {}\n> {} {} [[Note {}]]",
                    pick(),
                    pick(),
                    pick(),
                    pick(),
                    n
                );
                n += 2;
            }
            12..=15 => {
                let _ = writeln!(
                    out,
                    "- [ ] {} {} #{} [[Note {}|{}]]",
                    pick(),
                    pick(),
                    pick(),
                    n % 97,
                    pick()
                );
                n += 1;
            }
            20 => {
                let _ = writeln!(out, "```rust\nfn {}() {{ // [[not a link]] }}\n```", pick());
                n += 3;
            }
            _ => {
                let _ = writeln!(
                    out,
                    "{} {} **{}** {} [[Note {}#{}]] {} `{}` #{}/{} {} *{}* =={}== {}.",
                    pick(),
                    pick(),
                    pick(),
                    pick(),
                    n % 211,
                    pick(),
                    pick(),
                    pick(),
                    pick(),
                    pick(),
                    pick(),
                    pick(),
                    pick(),
                    pick()
                );
                n += 1;
            }
        }
    }
    out
}

#[derive(Clone)]
struct Args {
    vault: Option<PathBuf>,
    note: Option<String>,
    synthetic: Option<usize>,
    /// Number of self-driven edit and navigation steps to run before quitting.
    stress: Option<usize>,
    /// Pre-filled Search query, for screenshots.
    search: Option<String>,
    /// Open the quick switcher with this query, for screenshots.
    switcher: Option<String>,
    /// Right-dock tab to show: backlinks, outgoing, tags, properties, outline.
    pane: Option<String>,
    /// Open the graph view: `global` or `local`.
    graph: Option<String>,
    /// Open the command palette with this query, for screenshots.
    palette: Option<String>,
    /// Open Settings on this tab (`hotkeys`, `appearance`, ...).
    settings: Option<String>,
    /// Open the note in source mode.
    source: bool,
    /// Open the note in reading view, whatever `defaultViewMode` says.
    reading: bool,
    /// Open the find panel with this query.
    find: Option<String>,
    /// Open the find panel with this query and select every match (Find all).
    find_all: Option<String>,
    /// Rename the opened note to this name at startup (links rewritten).
    rename: Option<String>,
    /// Open a file prompt for the opened note: rename, move, delete, folder.
    prompt: Option<String>,
    /// Type this text at the cursor in the source editor, which is how the
    /// autocompletion popup is opened for a screenshot (`--type "[[al"`).
    type_text: Option<String>,
    /// Press these keys, comma separated, after `--type`: `enter`, `tab`,
    /// `shift-tab`, `shift-enter`, `escape`, `up`, `down`.
    accept: Option<String>,
    /// Open this cell of the note's first table for editing, before
    /// `--type`, the way clicking one does.
    cell: Option<usize>,
    /// Run these command ids at startup, comma separated, after everything
    /// else: `--command editor:table-row-after`.
    command: Option<String>,
    /// Open the template chooser and pick the template this matches.
    template: Option<String>,
    /// Open the Settings dropdown that writes this key, for screenshots.
    menu: Option<String>,
    /// Bind the control socket at this path, so something outside the app
    /// can read its state and drive it.
    control: Option<PathBuf>,
    /// Window size in logical pixels, `--size 1400x900`.
    size: Option<(f32, f32)>,
    /// `hidden` (the default, Obsidian's on Linux) or `native`.
    frame: Option<String>,
}

/// `1400x900` as a width and a height; anything else leaves the default.
fn parse_size(value: &str) -> Option<(f32, f32)> {
    let (width, height) = value.split_once(['x', 'X'])?;
    Some((width.trim().parse().ok()?, height.trim().parse().ok()?))
}

/// Decode `%XX` sequences in a URI component.
fn percent_decode(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
            {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

/// Parse an `obsidian://action?key=value&key=value` URI into action + params.
fn parse_obsidian_uri(uri: &str) -> Option<(String, Vec<(String, String)>)> {
    let rest = uri.strip_prefix("obsidian://")?;
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let action = percent_decode(path.trim_end_matches('/'));
    let params: Vec<(String, String)> = if query.is_empty() {
        Vec::new()
    } else {
        query
            .split('&')
            .filter_map(|pair| {
                let (k, v) = pair.split_once('=')?;
                Some((percent_decode(k), percent_decode(v)))
            })
            .collect()
    };
    Some((action, params))
}

/// Resolve a vault name from an obsidian:// URI to a filesystem path using
/// the vault registry, falling back to the most recently opened vault.
fn resolve_vault_by_name(name: &str) -> Option<PathBuf> {
    let reg = silica_vault::VaultRegistry::open().ok()??;
    for (_, entry) in reg.vaults() {
        if entry.name().eq_ignore_ascii_case(name) {
            return Some(entry.path.clone());
        }
    }
    None
}

/// Apply an `obsidian://` URI to `args`, resolving the vault name and setting
/// note/search fields as appropriate.
fn apply_obsidian_uri(args: &mut Args, uri: &str) {
    let Some((action, params)) = parse_obsidian_uri(uri) else {
        return;
    };
    let get = |key: &str| -> Option<String> {
        params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    };

    // Resolve vault from the URI's `vault` param.
    if let Some(vault_name) = get("vault") {
        if let Some(path) = resolve_vault_by_name(&vault_name) {
            args.vault = Some(path);
        }
    }

    match action.as_str() {
        "open" => {
            args.note = get("file");
        }
        "search" => {
            args.search = get("query");
        }
        "new" => {
            // `new` sets a note path; content is written by the workspace once
            // create-on-open is implemented.  For now, opening the path is the
            // best effort.
            args.note = get("file");
        }
        _ => {}
    }
}

fn print_help() {
    let version = env!("CARGO_PKG_VERSION");
    eprintln!(
        "Silica {version} — a native Obsidian-compatible knowledge base\n\
         \n\
         USAGE:\n\
         \x20   silica [OPTIONS] [<vault>] [<note>]\n\
         \x20   silica [OPTIONS] obsidian://open?vault=name&file=note\n\
         \n\
         POSITIONAL ARGUMENTS:\n\
         \x20   <vault>              Path to an Obsidian vault folder\n\
         \x20   <note>               Note name or path to open\n\
         \n\
         URI SCHEME:\n\
         \x20   obsidian://open?vault=name&file=path   Open a note\n\
         \x20   obsidian://search?vault=name&query=q   Open search\n\
         \x20   obsidian://new?vault=name&file=path    Create a note\n\
         \n\
         OPTIONS:\n\
         \x20   --note <path>        Open this note (name, path, or Name#Heading)\n\
         \x20   --search <query>     Open search with this query\n\
         \x20   --source             Open in source editing mode\n\
         \x20   --reading            Open in reading view\n\
         \x20   --graph [local]      Open the graph view (global or local)\n\
         \x20   --find <query>       Open the find panel\n\
         \x20   --settings [tab]     Open Settings on this tab\n\
         \x20   --frame hidden|native  Window frame style (default: hidden)\n\
         \x20   --size WxH           Window size in logical pixels (default: 1400x900)\n\
         \x20   --control <socket>   Bind the control socket at this path\n\
         \x20   --synthetic <N>      Generate a synthetic note of N lines\n\
         \x20   --stress [<N>]       Run N self-driven edit steps (default: 600)\n\
         \x20   --help               Print this help and exit\n\
         \x20   --version            Print version and exit"
    );
}

fn parse_args() -> Args {
    let mut args = Args {
        vault: None,
        note: None,
        synthetic: None,
        stress: None,
        search: None,
        switcher: None,
        pane: None,
        graph: None,
        palette: None,
        settings: None,
        source: false,
        reading: false,
        find: None,
        find_all: None,
        rename: None,
        prompt: None,
        type_text: None,
        accept: None,
        cell: None,
        command: None,
        template: None,
        menu: None,
        control: None,
        size: None,
        frame: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            "--version" | "-V" => {
                eprintln!("silica {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--synthetic" => {
                args.synthetic = it.next().and_then(|n| n.parse().ok()).or(Some(50_000))
            }
            "--stress" => args.stress = it.next().and_then(|n| n.parse().ok()).or(Some(600)),
            "--note" => args.note = it.next(),
            "--search" => args.search = it.next(),
            "--switcher" => args.switcher = it.next(),
            "--pane" => args.pane = it.next(),
            "--graph" => args.graph = it.next().or(Some("global".into())),
            "--palette" => args.palette = it.next().or(Some(String::new())),
            "--settings" => args.settings = it.next().or(Some("general".into())),
            "--source" => args.source = true,
            "--reading" => args.reading = true,
            "--find" => args.find = it.next().or(Some(String::new())),
            "--find-all" => args.find_all = it.next().or(Some(String::new())),
            "--rename" => args.rename = it.next(),
            "--prompt" => args.prompt = it.next().or(Some("rename".into())),
            "--type" => args.type_text = it.next(),
            "--accept" | "--press" => args.accept = it.next().or(Some("enter".into())),
            "--cell" => args.cell = it.next().and_then(|n| n.parse().ok()),
            "--command" => args.command = it.next(),
            "--template" => args.template = it.next(),
            "--menu" => args.menu = it.next(),
            "--control" => args.control = it.next().map(PathBuf::from),
            "--size" => args.size = it.next().as_deref().and_then(parse_size),
            "--frame" => args.frame = it.next(),
            _ if a.starts_with("obsidian://") => {
                apply_obsidian_uri(&mut args, &a);
            }
            _ if args.vault.is_none() => args.vault = Some(PathBuf::from(a)),
            _ => args.note = Some(a),
        }
    }
    args
}

/// The "Window frame style" the app was started with, as Obsidian's
/// Interface tab labels it: "Hidden (default)", "Obsidian frame" or
/// "Native frame". Obsidian keeps the choice outside the vault, so it is an
/// argument here (`--frame hidden|native`, or `SILICA_FRAME`).
pub fn frame_style_label() -> &'static str {
    match frame_style().as_str() {
        "native" => "Native frame",
        "custom" => "Obsidian frame",
        _ => "Hidden (default)",
    }
}

/// Whether the window is opened without the desktop's own title bar, which
/// is what Obsidian's "Hidden" frame style means and its default on Linux
/// and Windows. Answered once: the window asks on the way up, and the
/// workspace asks again on every frame.
pub fn hidden_frame() -> bool {
    static HIDDEN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *HIDDEN.get_or_init(|| frame_style() != "native")
}

/// `--frame`'s value, `SILICA_FRAME`'s, or `hidden`.
fn frame_style() -> String {
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        if a == "--frame" {
            if let Some(v) = it.next() {
                return v;
            }
        }
    }
    std::env::var("SILICA_FRAME").unwrap_or_else(|_| "hidden".into())
}

/// Every key binding in the app, from scratch: the views' own and the
/// registry's hotkeys. Called at startup and again whenever `hotkeys.json`
/// changes, since a binding made from the old registry would otherwise stay.
pub fn bind_all_keys(cx: &mut App, registry: &CommandRegistry) {
    cx.clear_key_bindings();
    editor::bind_keys(cx);
    workspace::bind_keys(cx, registry);
    text_input::bind_keys(cx);
    graph::bind_keys(cx);
    canvas::view::bind_keys(cx);
    pdf::view::bind_keys(cx);
    explorer::bind_keys(cx);
    settings::bind_keys(cx);
    help::bind_keys(cx);
    vaults::bind_keys(cx);
    prompt::bind_keys(cx);
    copy_results::bind_keys(cx);
    cx.bind_keys([KeyBinding::new("ctrl-q", Quit, None)]);
}

fn main() {
    let args = parse_args();
    // The theme comes from appearance.json and the key bindings from
    // hotkeys.json; read both before the window opens so the first frame
    // already has the right colors and keys.
    let vault = args
        .vault
        .as_ref()
        .and_then(|root| silica_vault::Vault::open(root).ok())
        .or_else(|| {
            silica_vault::VaultRegistry::open()
                .ok()
                .flatten()
                .and_then(|reg| {
                    reg.vaults()
                        .into_iter()
                        .max_by_key(|(_, e)| e.ts)
                        .map(|(_, e)| e.path.clone())
                })
                .and_then(|path| silica_vault::Vault::open(&path).ok())
        });
    // A community CSS theme layers its custom properties on top of the base
    // theme; read it from `.obsidian/themes/<name>.css` inside the vault.
    let css_theme = vault
        .as_ref()
        .and_then(|v| v.appearance_config().ok().and_then(|a| a.css_theme.clone()))
        .and_then(|name| {
            vault.as_ref().and_then(|v| {
                let root = v.root().to_path_buf();
                theme::load_css_theme(&root, &name)
            })
        });
    // Enabled CSS snippets from `.obsidian/snippets/<name>.css`; applied last
    // so they override both base and community theme colors.
    let css_snippets = vault
        .as_ref()
        .and_then(|v| {
            v.appearance_config()
                .ok()
                .and_then(|a| a.enabled_css_snippets.clone())
        })
        .unwrap_or_default();
    let css_snippets = vault
        .as_ref()
        .and_then(|v| {
            let root = v.root().to_path_buf();
            Some(theme::load_css_snippets(&root, &css_snippets))
        })
        .unwrap_or_default();
    // The window is what knows the desktop's colour scheme, and it does not
    // exist yet; dark is assumed here and the theme is rebuilt below once
    // the window can answer, which only matters for `theme: "system"`.
    let appearance = vault.as_ref().and_then(|v| v.appearance_config().ok());
    let theme = appearance
        .as_ref()
        .map(|a| Theme::from_appearance(a, true, css_theme.as_ref(), &css_snippets))
        .unwrap_or_else(Theme::dark);
    let mut registry = CommandRegistry::with_known_ids();
    // A core plugin that is off has no commands: not in the palette, and
    // nothing bound to its keys. `core-plugins.json` is read here so the
    // first frame is already right, and `change_setting` rebuilds this the
    // same way when a toggle changes it.
    if let Some(vault) = vault.as_ref() {
        let core = vault.core_plugins().unwrap_or_default();
        registry.retain_enabled(|id| {
            silica_formats::config::CORE_PLUGINS
                .iter()
                .find(|p| p.id == id)
                .is_none_or(|p| {
                    silica_formats::config::CorePlugins::is_enabled_or_default(core.as_ref(), p)
                })
        });
    }
    if let Some(hotkeys) = vault.as_ref().and_then(|v| v.hotkeys().ok()) {
        registry.apply_overrides(&hotkeys);
    }
    drop(vault);
    let registry = Arc::new(registry);

    // Obsidian's "Window frame style" is "Hidden" on Linux and Windows: no
    // title bar of the desktop's, the tab strip at the window's top edge and
    // the window buttons in its right corner. `--frame native` asks the
    // desktop for its own frame instead, which is the choice for a
    // compositor that draws client windows badly.
    let frame = args.frame.clone().unwrap_or_else(frame_style);
    let decorations = if frame == "native" {
        WindowDecorations::Server
    } else {
        WindowDecorations::Client
    };

    gpui_platform::application()
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            cx.set_http_client(Arc::new(http_client::SilicaHttpClient::new()));
            cx.set_global(theme);
            if let Err(e) = cx.text_system().add_fonts(assets::fonts()) {
                eprintln!("could not register the bundled fonts: {e}");
            }
            bind_all_keys(cx, &registry);
            cx.set_global(Commands(registry));

            let (width, height) = args.size.unwrap_or((1400.0, 900.0));
            let bounds = Bounds::centered(None, size(px(width), px(height)), cx);
            let vault = args.vault.clone();
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        titlebar: Some(TitlebarOptions {
                            title: Some("Silica".into()),
                            // What hides the caption on Windows and macOS,
                            // where `window_decorations` is not read at all:
                            // the window keeps the system's shadow and resize
                            // border and loses only the title bar, which is
                            // the frame Obsidian draws there. macOS keeps its
                            // traffic lights, at the point Obsidian's window
                            // options put them; the workspace pads the header
                            // under them (see `frame_left_space`).
                            appears_transparent: (cfg!(target_os = "windows")
                                || cfg!(target_os = "macos"))
                                && frame != "native",
                            traffic_light_position: (cfg!(target_os = "macos")
                                && frame != "native")
                                .then(|| point(px(19.), px(12.))),
                        }),
                        // KWin only honours the xdg-activation token that
                        // `activate_window` asks for when it can tie it to an
                        // application, which is what this id is for.
                        app_id: Some("silica".into()),
                        window_decorations: Some(decorations),
                        // A client-drawn frame paints its shadow in a margin
                        // around the content, which has to be see-through.
                        // Only Linux draws that margin: Windows keeps the
                        // system's shadow outside the window, so a
                        // transparent background there would only show the
                        // desktop through the app's own rounded corners.
                        window_background: match decorations {
                            WindowDecorations::Client if !cfg!(target_os = "windows") => {
                                WindowBackgroundAppearance::Transparent
                            }
                            _ => WindowBackgroundAppearance::Opaque,
                        },
                        window_min_size: Some(size(px(480.), px(320.))),
                        ..Default::default()
                    },
                    |window, cx| cx.new(|cx| Workspace::new(vault, window, cx)),
                )
                .expect("open main window");
            // Now there is a window to ask: `theme: "system"` follows the
            // desktop, which is Obsidian's default and what a vault with no
            // theme key means.
            if let Some(a) = appearance {
                let follows_system =
                    !matches!(a.theme.as_deref(), Some("moonstone") | Some("obsidian"));
                if follows_system {
                    let dark = window
                        .update(cx, |_, window, _| {
                            matches!(
                                window.appearance(),
                                WindowAppearance::Dark | WindowAppearance::VibrantDark
                            )
                        })
                        .unwrap_or(true);
                    cx.set_global(Theme::from_appearance(
                        &a,
                        dark,
                        css_theme.as_ref(),
                        &css_snippets,
                    ));
                    window.update(cx, |_, _, cx| cx.notify()).ok();
                }
            }
            cx.on_action(move |_: &Quit, cx| {
                // Save the note and the layout before the window goes away.
                window.update(cx, |ws, _, cx| ws.on_quit(cx)).ok();
                control::cleanup();
                cx.quit();
            });
            // Ctrl+Q is not the only way out: closing the window from the
            // titlebar quits too, and a socket file left behind makes the next
            // run look like one that is already listening.
            cx.on_app_quit(|_| {
                control::cleanup();
                async {}
            })
            .detach();

            // Bound before the startup flags are applied, so `--control`
            // composes with `--note`, `--source` and the rest of them.
            if let Some(path) = args.control.clone() {
                control::start(path, window, cx);
            }

            let note = args.note.clone();
            let search = args.search.clone();
            let switcher = args.switcher.clone();
            let pane = args.pane.clone();
            let graph = args.graph.clone();
            let palette = args.palette.clone();
            let settings_tab = args.settings.clone();
            let source = args.source;
            let type_text = args.type_text.clone();
            let accept = args.accept.clone();
            let cell = args.cell;
            let command = args.command.clone();
            let template = args.template.clone();
            let menu = args.menu.clone();
            let reading = args.reading;
            let find = args.find.clone();
            let find_all = args.find_all.clone();
            let rename = args.rename.clone();
            let prompt = args.prompt.clone();
            let synthetic = args.synthetic;
            window
                .update(cx, |ws, window, cx| {
                    if let Some(lines) = synthetic {
                        let text = synthetic_note(lines);
                        ws.editor().update(cx, |editor, cx| {
                            editor.set_text(format!("synthetic-{lines}.md"), text, cx)
                        });
                        ws.set_mode(ViewMode::Source, window, cx);
                        // The flag loop below never runs for a synthetic note, so
                        // this is as arranged as the window is going to get.
                        ws.mark_started();
                    } else {
                        window.focus(&ws.focus_handle(cx), cx);
                    }
                })
                .expect("window update");
            cx.activate(true);

            // The vault indexes in the background; open the requested note once it lands.
            if synthetic.is_none() {
                cx.spawn(async move |cx| loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(16))
                        .await;
                    let done = window
                        .update(cx, |ws, window, cx| {
                            if !ws.is_loaded() {
                                return false;
                            }
                            // The vault's default view mode, unless a flag says
                            // otherwise — or `workspace.json` restored the tabs,
                            // each in the mode it recorded.
                            let mode = if source {
                                Some(ViewMode::Source)
                            } else if reading {
                                Some(ViewMode::Reading)
                            } else if ws.layout_restored() {
                                None
                            } else {
                                Some(ws.default_mode())
                            };
                            if let Some(mode) = mode {
                                ws.set_mode(mode, window, cx);
                            }
                            // With a layout restored and no note asked for, the
                            // window stays as `workspace.json` left it, which is
                            // the empty state when nothing was open.
                            let opened = match &note {
                                Some(n) => ws.open_by_name(n, cx),
                                None => false,
                            };
                            if !opened && !ws.layout_restored() {
                                ws.open_first(cx);
                            }
                            if let Some(q) = &search {
                                ws.show_search(q, Some(window), cx);
                            }
                            if let Some(q) = &switcher {
                                ws.open_switcher(q, window, cx);
                            }
                            if let Some(p) = &pane {
                                ws.show_right_tab(p, cx);
                            }
                            if let Some(g) = &graph {
                                ws.show_graph(g == "local", window, cx);
                            }
                            if let Some(q) = &palette {
                                ws.open_palette(q, window, cx);
                            }
                            if let Some(t) = &settings_tab {
                                ws.open_settings(settings::Tab::parse(t), window, cx);
                            }
                            if let Some(q) = &find {
                                ws.open_find(q, false, window, cx);
                            }
                            if let Some(q) = &find_all {
                                ws.open_find(q, false, window, cx);
                                ws.editor()
                                    .update(cx, |e, cx| e.find_all_matches(window, cx));
                            }
                            if let Some(name) = &rename {
                                ws.rename_current(name, window, cx);
                            }
                            if let Some(kind) = &prompt {
                                ws.open_file_prompt(kind, window, cx);
                            }
                            if let Some(ix) = cell {
                                ws.editor().update(cx, |e, cx| e.edit_cell(ix, window, cx));
                            }
                            if let Some(t) = &type_text {
                                ws.editor().update(cx, |e, cx| e.type_text(t, window, cx));
                            }
                            if let Some(keys) = &accept {
                                ws.editor()
                                    .update(cx, |e, cx| e.press_keys(keys, window, cx));
                            }
                            if let Some(ids) = &command {
                                for id in ids.split(',') {
                                    ws.run_command(id.trim(), window, cx);
                                }
                            }
                            if let Some(q) = &template {
                                ws.pick_template(q, window, cx);
                            }
                            if let Some(key) = &menu {
                                ws.open_settings_menu(key, cx);
                            }
                            ws.mark_started();
                            true
                        })
                        .unwrap_or(true);
                    if done {
                        break;
                    }
                })
                .detach();
            }

            let stress = match args.stress {
                Some(_) if args.control.is_some() => {
                    eprintln!("--stress is ignored with --control: the socket drives the app");
                    None
                }
                steps => steps,
            };
            if let Some(steps) = stress {
                cx.spawn(async move |cx| {
                    // Without `--synthetic` the run edits the opened note (and
                    // saves it): wait for the vault to index and the note to open.
                    while synthetic.is_none() {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(50))
                            .await;
                        let ready = window
                            .update(cx, |ws, window, cx| {
                                let open = ws.is_loaded() && ws.current().is_some();
                                if open && !ws.mode().is_editing() {
                                    ws.set_mode(ViewMode::Source, window, cx);
                                }
                                open
                            })
                            .unwrap_or(true);
                        if ready {
                            break;
                        }
                    }
                    let started = std::time::Instant::now();
                    for step in 0..steps {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(8))
                            .await;
                        let _ = window.update(cx, |ws, window, cx| {
                            ws.editor()
                                .update(cx, |editor, cx| editor.stress_step(step, window, cx));
                        });
                    }
                    // Let the last parse land (and, on a real note, the autosave
                    // fire) before reporting.
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(if synthetic.is_some() {
                            1500
                        } else {
                            3000
                        }))
                        .await;
                    let report = window
                        .update(cx, |ws, _, cx| ws.editor().read(cx).report())
                        .unwrap_or_default();
                    eprintln!(
                        "stress: {steps} steps in {:.1} s\n{report}",
                        started.elapsed().as_secs_f64()
                    );
                    cx.update(|cx| cx.quit());
                })
                .detach();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decode_handles_hex_escapes_and_passes_unknown_through() {
        assert_eq!(percent_decode("hello%20world"), "hello world");
        assert_eq!(percent_decode("a%2Fb%2Cc"), "a/b,c");
        assert_eq!(percent_decode("not%20%20encoded"), "not  encoded");
        // Invalid % sequences are not touched: leave the literal characters.
        assert_eq!(percent_decode("100% GG"), "100% GG");
        assert_eq!(percent_decode("trailing%"), "trailing%");
        assert_eq!(percent_decode("plain"), "plain");
        assert_eq!(percent_decode(""), "");
    }

    #[test]
    fn parse_obsidian_uri_splits_action_and_percent_decoded_params() {
        let (action, params) =
            parse_obsidian_uri("obsidian://open?vault=My%20Vault&file=Note%23Heading.md").unwrap();
        assert_eq!(action, "open");
        assert_eq!(
            params,
            vec![
                ("vault".into(), "My Vault".into()),
                ("file".into(), "Note#Heading.md".into())
            ]
        );

        // No query -> empty params, trailing slash on action is trimmed.
        let (action, params) = parse_obsidian_uri("obsidian://search/").unwrap();
        assert_eq!(action, "search");
        assert!(params.is_empty());

        // `obsidian://` with an empty action is still a valid parse.
        assert_eq!(parse_obsidian_uri("obsidian://"), Some(("".into(), vec![])));

        // Not an obsidian URI -> None.
        assert!(parse_obsidian_uri("https://example.com/x").is_none());
        assert!(parse_obsidian_uri("").is_none());
    }

    #[test]
    fn apply_obsidian_uri_sets_note_and_search_without_crashing_on_unknown_action() {
        let base = Args {
            vault: None,
            note: None,
            synthetic: None,
            stress: None,
            search: None,
            switcher: None,
            pane: None,
            graph: None,
            palette: None,
            settings: None,
            source: false,
            reading: false,
            find: None,
            find_all: None,
            rename: None,
            prompt: None,
            type_text: None,
            accept: None,
            cell: None,
            command: None,
            template: None,
            menu: None,
            control: None,
            size: None,
            frame: None,
        };

        let mut args = base.clone();
        apply_obsidian_uri(&mut args, "obsidian://open?vault=V&file=My Note.md");
        assert_eq!(args.note.as_deref(), Some("My Note.md"));

        let mut args = base.clone();
        apply_obsidian_uri(&mut args, "obsidian://search?vault=V&query=hello+world");
        assert_eq!(args.search.as_deref(), Some("hello+world"));

        let mut args = base.clone();
        apply_obsidian_uri(&mut args, "obsidian://do-anything?vault=V&file=x.md");
        assert!(args.note.is_none());
        assert!(args.vault.is_none());
    }
}
