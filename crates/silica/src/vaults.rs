//! The vault manager, what "Manage vaults..." and `app:open-vault` raise:
//! the vaults this machine has opened on the left, and what to do about a
//! vault that is not on the list on the right.
//!
//! Obsidian's is a window of its own rather than a modal
//! (`app://obsidian.md/starter.html`, 800 by 650, read over CDP on
//! 2026-10-29 by `tools/obsidian-probe/vault-manager.mjs`): a
//! `.recent-vaults` column 280 px wide beside a 520 px `.splash` holding
//! the mark, the name, the version and a card of rows. A row of the list
//! is the folder's **name** over the **parent folder's** path, with the
//! full path as its tooltip and an ellipsis at the right ("More options")
//! opening a menu. The list is `ts` descending — most recently opened
//! first — which is *not* the switcher menu's order (that one is the
//! registry document's key order), and a vault whose folder is gone is
//! left out of both.
//!
//! Silica draws the same window with its own three rows on the right:
//! Obsidian's third is "Open vault from Obsidian Sync", which Silica has
//! no answer to, so the sandbox vault takes that place. Obsidian's fourth
//! row is a language dropdown; Silica is English only and has none.

use crate::assets::{icon, logo};
use crate::theme::{Theme, FONT_UI_MEDIUM, FONT_UI_SMALL, FONT_UI_SMALLER};
use crate::workspace::Workspace;
use gpui::{
    actions, anchored, deferred, div, point, prelude::*, px, size, AnyWindowHandle, App, Bounds,
    Context, FocusHandle, Focusable, FontWeight, KeyBinding, SharedString, TitlebarOptions,
    WeakEntity, Window, WindowBackgroundAppearance, WindowBounds, WindowDecorations, WindowHandle,
    WindowOptions,
};
use silica_vault::VaultRegistry;
use std::path::PathBuf;

actions!(vaults, [Close]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", Close, Some("Vaults"))]);
}

/// The window's size: Obsidian's own 800 by 650.
pub const WINDOW_SIZE: (f32, f32) = (800., 650.);

/// The list's column, Obsidian's `.recent-vaults` width.
const LIST_WIDTH: f32 = 280.;

/// One row of the right-hand card.
struct Choice {
    name: &'static str,
    blurb: &'static str,
    button: &'static str,
    /// Drawn in the accent, the one row Obsidian singles out (`mod-cta`).
    primary: bool,
    go: Go,
}

/// What a row's button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Go {
    Create,
    Open,
    Sandbox,
}

/// The rows, in Obsidian's order and with its wording where Silica does
/// the same thing. The third is Silica's own: Obsidian offers Sync there.
const OPTIONS: &[Choice] = &[
    Choice {
        name: "Create new vault",
        blurb: "Create a new Silica vault under a folder.",
        button: "Create",
        primary: true,
        go: Go::Create,
    },
    Choice {
        name: "Open folder as vault",
        blurb: "Choose an existing folder of Markdown files.",
        button: "Open",
        primary: false,
        go: Go::Open,
    },
    Choice {
        name: "Open sandbox vault",
        blurb: "Experiment in a fresh vault under the temporary folder.",
        button: "Open",
        primary: false,
        go: Go::Sandbox,
    },
];

/// A vault on the list, as a row draws it.
#[derive(Debug, Clone)]
struct Row {
    /// The registry's id for it, what "Copy vault ID" copies.
    id: SharedString,
    /// The folder's name, the row's heading.
    name: SharedString,
    /// The *parent* folder's path, the line under the name.
    parent: SharedString,
    /// The folder itself, the row's tooltip and what opening it loads.
    path: PathBuf,
}

/// What the ellipsis opens. Obsidian's menu has seven rows; Silica draws
/// the three that do not move the user's folders about — "Rename vault..."
/// and "Move vault..." rename and move the folder on disk and are not
/// built yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowItem {
    CopyId,
    Reveal,
    Forget,
}

impl RowItem {
    fn label(self) -> &'static str {
        match self {
            RowItem::CopyId => "Copy vault ID",
            RowItem::Reveal => "Reveal vault in system explorer",
            RowItem::Forget => "Remove from list",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            RowItem::CopyId => "copy",
            RowItem::Reveal => "folder-open",
            RowItem::Forget => "x",
        }
    }

    /// Obsidian draws "Remove from list" `is-warning`.
    fn warning(self) -> bool {
        self == RowItem::Forget
    }
}

/// The menu's rows, with a separator before each group, as Obsidian's are.
const ROW_ITEMS: &[(RowItem, bool)] = &[
    (RowItem::CopyId, false),
    (RowItem::Reveal, true),
    (RowItem::Forget, true),
];

pub struct VaultManager {
    focus_handle: FocusHandle,
    /// The workspace that opened it: the window a picked vault loads into.
    workspace: WeakEntity<Workspace>,
    main: AnyWindowHandle,
    rows: Vec<Row>,
    /// The row whose "More options" menu is open.
    menu: Option<usize>,
    /// Whether the pointer is over that row's ellipsis: the menu closes on
    /// a mouse down outside itself, which a click on the ellipsis is, and
    /// without this the press would close the menu the click reopens.
    over_dots: Option<usize>,
    /// What went wrong, under the list.
    error: Option<SharedString>,
}

impl VaultManager {
    /// Opens the window beside `main`, the workspace's own, which is the
    /// window a vault picked here is loaded into.
    pub fn open(
        workspace: WeakEntity<Workspace>,
        main: AnyWindowHandle,
        cx: &mut App,
    ) -> anyhow::Result<WindowHandle<VaultManager>> {
        let (w, h) = WINDOW_SIZE;
        let bounds = Bounds::centered(None, size(px(w), px(h)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Vaults".into()),
                    ..Default::default()
                }),
                app_id: Some("silica".into()),
                // The desktop's own frame, as the Help window has: a
                // dialog has no tab strip to put window buttons in.
                window_decorations: Some(WindowDecorations::Server),
                window_background: WindowBackgroundAppearance::Opaque,
                window_min_size: Some(size(px(640.), px(480.))),
                ..Default::default()
            },
            move |window, cx| {
                let view = cx.new(|cx| VaultManager {
                    focus_handle: cx.focus_handle(),
                    workspace,
                    main,
                    rows: read_rows(),
                    menu: None,
                    over_dots: None,
                    error: None,
                });
                let handle = view.read(cx).focus_handle.clone();
                window.focus(&handle, cx);
                view
            },
        )
    }

    /// Reads the list again, after something changed it.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.rows = read_rows();
        self.menu = None;
        cx.notify();
    }

    fn report(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.error = Some(message.into());
        cx.notify();
    }

    /// Loads `root` in the workspace's window and closes this one.
    fn open_vault(&mut self, root: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if !root.is_dir() {
            self.report(format!("{} is not a folder any more", root.display()), cx);
            self.refresh(cx);
            return;
        }
        let workspace = self.workspace.clone();
        self.main
            .update(cx, |_, main_window, cx| {
                workspace
                    .update(cx, |ws, cx| ws.load_vault(root, main_window, cx))
                    .ok();
                main_window.activate_window();
            })
            .ok();
        window.remove_window();
    }

    fn go(&mut self, go: Go, window: &mut Window, cx: &mut Context<Self>) {
        match go {
            // The desktop's save dialog: the user says where the vault
            // goes and what the folder is called in one step.
            Go::Create => {
                let picked = cx.prompt_for_new_path(&home_dir(), Some("New vault"));
                cx.spawn_in(window, async move |this, cx| {
                    let root = match picked.await {
                        Ok(Ok(Some(path))) => path,
                        Ok(Ok(None)) => return,
                        Ok(Err(err)) => {
                            this.update(cx, |this, cx| {
                                this.report(format!("Could not open the dialog: {err}"), cx)
                            })
                            .ok();
                            return;
                        }
                        Err(_) => return,
                    };
                    let made = make_vault(&root);
                    this.update_in(cx, |this, window, cx| match made {
                        Ok(()) => this.open_vault(root, window, cx),
                        Err(err) => this.report(format!("Could not make the vault: {err}"), cx),
                    })
                    .ok();
                })
                .detach();
            }
            Go::Open => {
                let picked = cx.prompt_for_paths(gpui::PathPromptOptions {
                    files: false,
                    directories: true,
                    multiple: false,
                    prompt: Some("Open vault".into()),
                });
                cx.spawn_in(window, async move |this, cx| {
                    let root = match picked.await {
                        Ok(Ok(Some(mut paths))) if !paths.is_empty() => paths.remove(0),
                        Ok(Ok(_)) => return,
                        Ok(Err(err)) => {
                            this.update(cx, |this, cx| {
                                this.report(format!("Could not open the folder dialog: {err}"), cx)
                            })
                            .ok();
                            return;
                        }
                        Err(_) => return,
                    };
                    this.update_in(cx, |this, window, cx| this.open_vault(root, window, cx))
                        .ok();
                })
                .detach();
            }
            Go::Sandbox => {
                let workspace = self.workspace.clone();
                self.main
                    .update(cx, |_, main_window, cx| {
                        workspace
                            .update(cx, |ws, cx| ws.open_sandbox_vault(main_window, cx))
                            .ok();
                        main_window.activate_window();
                    })
                    .ok();
                window.remove_window();
            }
        }
    }

    /// One row of the ellipsis menu.
    fn run_item(&mut self, ix: usize, item: RowItem, cx: &mut Context<Self>) {
        self.menu = None;
        let Some(row) = self.rows.get(ix).cloned() else {
            cx.notify();
            return;
        };
        match item {
            RowItem::CopyId => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(row.id.to_string()));
            }
            RowItem::Reveal => cx.reveal_path(&row.path),
            RowItem::Forget => match forget(&row.id) {
                Ok(()) => self.refresh(cx),
                Err(err) => self.report(format!("Could not change the list: {err}"), cx),
            },
        }
        cx.notify();
    }

    fn close(&mut self, _: &Close, window: &mut Window, _: &mut Context<Self>) {
        window.remove_window();
    }

    /// A row of the list: the folder's name over its parent's path, with
    /// the ellipsis at the right.
    fn render_row(
        &self,
        ix: usize,
        row: &Row,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let path = row.path.clone();
        let open = self.menu == Some(ix);
        div()
            .id(("vault-row", ix))
            .relative()
            .flex()
            .flex_row()
            .items_start()
            .gap(px(4.))
            .px(px(12.))
            .py(px(8.))
            .rounded(px(6.))
            .cursor_pointer()
            .hover(|s| s.bg(theme.bg_alt))
            .when(open, |d| d.bg(theme.bg_alt))
            .tooltip(crate::canvas::menu::tooltip(row.path.display().to_string()))
            .on_click(
                cx.listener(move |this, _, window, cx| this.open_vault(path.clone(), window, cx)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(FONT_UI_MEDIUM))
                            .text_color(theme.text)
                            .child(row.name.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(FONT_UI_SMALLER))
                            .text_color(theme.text_muted)
                            .child(row.parent.clone()),
                    ),
            )
            .child(
                div()
                    .id(("vault-row-options", ix))
                    .flex_shrink_0()
                    .size(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.bg_active))
                    .tooltip(crate::canvas::menu::tooltip("More options"))
                    .on_hover(cx.listener(move |this, over: &bool, _, _| {
                        this.over_dots = if *over { Some(ix) } else { None };
                    }))
                    // The row under it opens the vault: a click on the
                    // ellipsis must not do both.
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.menu = if this.menu == Some(ix) {
                            None
                        } else {
                            Some(ix)
                        };
                        cx.stop_propagation();
                        cx.notify();
                    }))
                    .child(icon("ellipsis-vertical", 16., theme.text_muted)),
            )
            // The menu's anchor is this box, pinned at the row's top
            // right corner, where Obsidian drops it from the ellipsis: an
            // absolute element left to the row's own flow would hang off
            // whatever the last child laid out to.
            .child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .size_0()
                    .children(self.render_row_menu(ix, theme, cx)),
            )
    }

    /// "More options": Copy vault ID, Reveal vault in system explorer,
    /// Remove from list.
    fn render_row_menu(
        &self,
        ix: usize,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if self.menu != Some(ix) {
            return None;
        }
        let mut list = div()
            .id(("vault-row-menu", ix))
            .flex()
            .flex_col()
            .min_w(px(240.))
            .py_1()
            .bg(theme.bg_secondary)
            .border_1()
            .border_color(theme.border)
            .rounded_md()
            .shadow_lg()
            .text_size(px(FONT_UI_SMALL))
            .text_color(theme.text)
            .occlude()
            .on_mouse_down_out(cx.listener(move |this, _, _, cx| {
                // The ellipsis closes the menu itself, on the click that
                // ends the same press.
                if this.over_dots == Some(ix) {
                    return;
                }
                this.menu = None;
                cx.notify();
            }));
        for (n, (item, separator)) in ROW_ITEMS.iter().copied().enumerate() {
            if separator {
                list = list.child(div().my_1().h(px(1.)).bg(theme.border));
            }
            // Obsidian draws "Remove from list" in `--text-error`, which
            // its default theme takes from `--color-red`: the first of the
            // canvas presets Silica already reads from the same place.
            let color = if item.warning() {
                theme.canvas.presets[0]
            } else {
                theme.text
            };
            list = list.child(
                div()
                    .id(("vault-row-menu-item", ix * ROW_ITEMS.len() + n))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py(px(5.))
                    .mx_1()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .text_color(color)
                    .hover(|s| s.bg(theme.bg_alt))
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.run_item(ix, item, cx);
                    }))
                    .child(icon(item.icon(), 14., color))
                    .child(item.label()),
            );
        }
        Some(
            deferred(
                anchored()
                    .anchor(gpui::Anchor::TopLeft)
                    .offset(point(px(4.), px(4.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(list),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    /// One row of the right-hand card.
    fn render_option(
        &self,
        option: &'static Choice,
        last: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let go = option.go;
        let (bg, fg, border) = if option.primary {
            (theme.accent, gpui::white(), theme.accent)
        } else {
            (theme.bg_alt, theme.text, theme.border)
        };
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(16.))
            .py(px(18.))
            .when(!last, |d| d.border_b_1().border_color(theme.border))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(FONT_UI_MEDIUM))
                            .text_color(theme.text)
                            .child(option.name),
                    )
                    .child(
                        div()
                            .text_size(px(FONT_UI_SMALL))
                            .text_color(theme.text_muted)
                            .child(option.blurb),
                    ),
            )
            .child(
                div()
                    .id(option.name)
                    .flex_shrink_0()
                    .px(px(16.))
                    .h(px(32.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .min_w(px(96.))
                    .rounded(px(6.))
                    .bg(bg)
                    .border_1()
                    .border_color(border)
                    .text_size(px(FONT_UI_SMALL))
                    .text_color(fg)
                    .cursor_pointer()
                    .hover(move |s| {
                        if option.primary {
                            s.opacity(0.9)
                        } else {
                            s.bg(theme.bg_active)
                        }
                    })
                    .on_click(cx.listener(move |this, _, window, cx| this.go(go, window, cx)))
                    .child(option.button),
            )
    }
}

impl Focusable for VaultManager {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for VaultManager {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let version: SharedString = concat!("Version ", env!("CARGO_PKG_VERSION")).into();
        let rows = self.rows.clone();
        let mut list = div()
            .id("vault-list")
            .flex()
            .flex_col()
            .gap(px(2.))
            .p(px(8.))
            .size_full()
            .overflow_y_scroll();
        if rows.is_empty() {
            list = list.child(
                div()
                    .px(px(12.))
                    .py(px(8.))
                    .text_size(px(FONT_UI_SMALL))
                    .text_color(theme.text_faint)
                    .child("No vaults opened yet"),
            );
        }
        for (ix, row) in rows.iter().enumerate() {
            list = list.child(self.render_row(ix, row, &theme, cx));
        }
        let mut card = div()
            .w_full()
            .max_w(px(460.))
            .px(px(24.))
            .rounded(px(12.))
            .bg(theme.bg_secondary)
            .flex()
            .flex_col();
        let count = OPTIONS.len();
        for (ix, option) in OPTIONS.iter().enumerate() {
            card = card.child(self.render_option(option, ix + 1 == count, &theme, cx));
        }
        div()
            .id("vault-manager")
            .key_context("Vaults")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::close))
            .size_full()
            .flex()
            .flex_row()
            .bg(theme.bg)
            .font_family(theme.ui_font.clone())
            .text_color(theme.text)
            .child(
                div()
                    .flex_shrink_0()
                    .w(px(LIST_WIDTH))
                    .h_full()
                    .flex()
                    .flex_col()
                    .bg(theme.bg_secondary)
                    .child(div().flex_1().min_h_0().child(list))
                    .children(self.error.clone().map(|message| {
                        div()
                            .px(px(12.))
                            .py(px(8.))
                            .border_t_1()
                            .border_color(theme.border)
                            .text_size(px(FONT_UI_SMALLER))
                            .text_color(theme.text_muted)
                            .child(message)
                    })),
            )
            .child(
                div()
                    .id("vault-splash")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .px(px(30.))
                    .py(px(24.))
                    .child(logo(90.))
                    .child(
                        div()
                            .mt(px(16.))
                            .text_size(px(26.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text)
                            .child("Silica"),
                    )
                    .child(
                        div()
                            .mt(px(4.))
                            .text_size(px(FONT_UI_SMALL))
                            .text_color(theme.text_muted)
                            .child(version),
                    )
                    .child(div().h(px(32.)))
                    .child(card),
            )
    }
}

/// The list the window draws: every vault the registry remembers whose
/// folder is still there, most recently opened first. Obsidian's manager
/// sorts by `ts` descending — unlike its switcher menu, which keeps the
/// document's key order — and leaves a vault whose folder is gone out
/// (probe of 2026-10-29). Entries with the same `ts` keep the document's
/// order, as Obsidian's did.
fn read_rows() -> Vec<Row> {
    let Some(registry) = VaultRegistry::open().ok().flatten() else {
        return Vec::new();
    };
    let mut rows: Vec<(i64, Row)> = registry
        .vaults()
        .into_iter()
        .filter(|(_, e)| e.path.is_dir())
        .map(|(id, e)| {
            (
                e.ts,
                Row {
                    id: id.to_string().into(),
                    name: e.name().into(),
                    parent: e
                        .path
                        .parent()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                        .into(),
                    path: e.path.clone(),
                },
            )
        })
        .collect();
    rows.sort_by_key(|(ts, _)| std::cmp::Reverse(*ts));
    rows.into_iter().map(|(_, row)| row).collect()
}

/// "Remove from list": the entry goes, the folder stays.
fn forget(id: &str) -> std::io::Result<()> {
    let Some(mut registry) = VaultRegistry::open()? else {
        return Ok(());
    };
    if registry.forget(id) {
        registry.save()?;
    }
    Ok(())
}

/// "Create new vault": the folder and the `.obsidian` beside it, which is
/// all a vault is. An existing folder is adopted rather than refused —
/// the dialog has already asked about overwriting.
fn make_vault(root: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(root.join(".obsidian"))
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Obsidian's manager window sorts the list by `ts` descending, where
    /// its switcher menu keeps the registry's key order (probe of
    /// 2026-10-29): the newest vault heads the list however late its key
    /// was written.
    #[test]
    fn the_list_is_most_recently_opened_first() {
        let dir = tempfile::tempdir().unwrap();
        let mut made = Vec::new();
        for name in ["first", "second", "third"] {
            let path = dir.path().join(name);
            std::fs::create_dir_all(&path).unwrap();
            made.push(path);
        }
        let registry_path = dir.path().join("silica.json");
        let doc = serde_json::json!({
            "vaults": {
                "aa00000000000001": { "path": made[0], "ts": 100 },
                "aa00000000000002": { "path": made[1], "ts": 300 },
                "aa00000000000003": { "path": made[2], "ts": 200 },
            }
        });
        std::fs::write(&registry_path, serde_json::to_vec(&doc).unwrap()).unwrap();
        let registry = VaultRegistry::load(&registry_path).unwrap();
        let mut rows: Vec<(i64, String)> = registry
            .vaults()
            .into_iter()
            .filter(|(_, e)| e.path.is_dir())
            .map(|(_, e)| (e.ts, e.name()))
            .collect();
        rows.sort_by_key(|(ts, _)| std::cmp::Reverse(*ts));
        let names: Vec<&str> = rows.iter().map(|(_, n)| n.as_str()).collect();
        assert_eq!(names, ["second", "third", "first"]);
    }

    /// A row is the folder's name over its *parent's* path, which is what
    /// Obsidian's `.recent-vaults-list-item-name` and `-path` hold.
    #[test]
    fn a_row_is_the_name_over_the_parent_folder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Field notes");
        std::fs::create_dir_all(&path).unwrap();
        let row = Row {
            id: "aa00000000000001".into(),
            name: path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
                .into(),
            parent: path.parent().unwrap().display().to_string().into(),
            path: path.clone(),
        };
        assert_eq!(row.name.as_ref(), "Field notes");
        assert_eq!(row.parent.as_ref(), dir.path().display().to_string());
        assert!(!row.parent.as_ref().ends_with("Field notes"));
    }

    /// "Create new vault" makes the folder and the `.obsidian` in it.
    #[test]
    fn creating_a_vault_writes_the_config_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("New vault");
        make_vault(&root).unwrap();
        assert!(root.join(".obsidian").is_dir());
    }
}
