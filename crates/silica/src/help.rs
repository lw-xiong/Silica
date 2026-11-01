//! The Help window, what the vault profile's `circle-help` button and
//! `app:open-help` open.
//!
//! Obsidian 1.14 answers "Open help" with a window of its own rather than
//! the browser: its mark, name and version over a card of four rows, each
//! an icon, a title, a line of explanation and a button on the right —
//! the help site (an accent "Visit"), its Discord ("Join"), the forum
//! ("Visit") and the sandbox vault ("Open"). Silica draws the same window
//! with its own four: the manual, the source, the issue tracker and the
//! sandbox vault. Escape or the frame's close button dismisses it, and
//! opening help again while it is up brings it forward rather than making
//! a second one.

use crate::assets::{icon, logo};
use crate::theme::{Theme, FONT_UI_MEDIUM};
use crate::workspace::Workspace;
use gpui::{
    actions, div, prelude::*, px, size, AnyWindowHandle, App, Bounds, Context, FocusHandle,
    Focusable, FontWeight, KeyBinding, SharedString, TitlebarOptions, WeakEntity, Window,
    WindowBackgroundAppearance, WindowBounds, WindowDecorations, WindowHandle, WindowOptions,
};

actions!(help, [Close]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", Close, Some("Help"))]);
}

/// The window's size: enough for the four rows at the text sizes below
/// without a scrollbar, and the shape of Obsidian's.
pub const WINDOW_SIZE: (f32, f32) = (700., 860.);

/// Where "Visit" goes for the manual: the project's GitHub repo, since
/// there is no separate manual site; the README is the manual.
pub const MANUAL_URL: &str = "https://github.com/lw-xiong/Silica";
pub const SOURCE_URL: &str = "https://github.com/lw-xiong/Silica";
pub const ISSUES_URL: &str = "https://github.com/lw-xiong/Silica/issues";

/// What a row's button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Go {
    Url(&'static str),
    Sandbox,
}

/// One row of the card.
struct Row {
    icon: &'static str,
    title: &'static str,
    blurb: &'static str,
    button: &'static str,
    /// Drawn in the accent, the one row Obsidian singles out.
    primary: bool,
    go: Go,
}

/// The rows, in Obsidian's order: help first, the sandbox last.
const ROWS: &[Row] = &[
    Row {
        icon: "book-open",
        title: "Manual",
        blurb: "Read the Silica manual: what the app does with a vault, and what it does not yet.",
        button: "Visit",
        primary: true,
        go: Go::Url(MANUAL_URL),
    },
    Row {
        icon: "code",
        title: "Source code",
        blurb: "Silica is open source. Read the code, build it yourself, or send a change.",
        button: "Visit",
        primary: false,
        go: Go::Url(SOURCE_URL),
    },
    Row {
        icon: "bug",
        title: "Report an issue",
        blurb: "Found something Obsidian does that Silica does not? Say which note shows it.",
        button: "Visit",
        primary: false,
        go: Go::Url(ISSUES_URL),
    },
    Row {
        icon: "flask-conical",
        title: "Sandbox vault",
        blurb: "Play around and experiment with various features. The sandbox is a fresh \
                folder in the temporary directory, so nothing you have is touched.",
        button: "Open",
        primary: false,
        go: Go::Sandbox,
    },
];

/// The Help window's root view.
pub struct HelpWindow {
    focus_handle: FocusHandle,
    /// The workspace that opened it, for the sandbox vault.
    workspace: WeakEntity<Workspace>,
    main: AnyWindowHandle,
}

impl HelpWindow {
    /// Opens the window next to `main`, the workspace's own, which is what
    /// the sandbox vault loads into.
    pub fn open(
        workspace: WeakEntity<Workspace>,
        main: AnyWindowHandle,
        cx: &mut App,
    ) -> anyhow::Result<WindowHandle<HelpWindow>> {
        let (w, h) = WINDOW_SIZE;
        let bounds = Bounds::centered(None, size(px(w), px(h)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Help".into()),
                    ..Default::default()
                }),
                app_id: Some("silica".into()),
                // The desktop's own frame: a dialog has no tab strip to
                // put the window buttons in.
                window_decorations: Some(WindowDecorations::Server),
                window_background: WindowBackgroundAppearance::Opaque,
                window_min_size: Some(size(px(480.), px(480.))),
                ..Default::default()
            },
            move |window, cx| {
                let view = cx.new(|cx| HelpWindow {
                    focus_handle: cx.focus_handle(),
                    workspace,
                    main,
                });
                let handle = view.read(cx).focus_handle.clone();
                window.focus(&handle, cx);
                view
            },
        )
    }

    fn go(&mut self, go: Go, window: &mut Window, cx: &mut Context<Self>) {
        match go {
            Go::Url(url) => cx.open_url(url),
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

    fn close(&mut self, _: &Close, window: &mut Window, _: &mut Context<Self>) {
        window.remove_window();
    }

    fn render_row(
        &self,
        row: &'static Row,
        last: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let go = row.go;
        let (bg, fg, border) = if row.primary {
            (theme.accent, gpui::white(), theme.accent)
        } else {
            (theme.bg_alt, theme.text, theme.border)
        };
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(24.))
            .py(px(22.))
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex_shrink_0()
                    .w(px(64.))
                    .flex()
                    .justify_center()
                    .child(icon(row.icon, 56., theme.text_faint)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .justify_center()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(17.))
                            .text_color(theme.text)
                            .child(row.title),
                    )
                    .child(
                        div()
                            .text_size(px(FONT_UI_MEDIUM))
                            .text_color(theme.text_muted)
                            .child(row.blurb),
                    ),
            )
            .child(
                div()
                    .id(row.title)
                    .flex_shrink_0()
                    .px(px(16.))
                    .h(px(36.))
                    .flex()
                    .items_center()
                    .rounded(px(8.))
                    .bg(bg)
                    .border_1()
                    .border_color(border)
                    .text_size(px(16.))
                    .text_color(fg)
                    .cursor_pointer()
                    .hover(move |s| {
                        if row.primary {
                            s.opacity(0.9)
                        } else {
                            s.bg(theme.bg_active)
                        }
                    })
                    .on_click(cx.listener(move |this, _, window, cx| this.go(go, window, cx)))
                    .child(row.button),
            )
    }
}

impl Focusable for HelpWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for HelpWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let version: SharedString = concat!("Version ", env!("CARGO_PKG_VERSION")).into();
        let mut card = div()
            .w_full()
            .max_w(px(720.))
            .px(px(24.))
            .rounded(px(12.))
            .bg(theme.bg_secondary)
            .flex()
            .flex_col();
        let count = ROWS.len();
        for (ix, row) in ROWS.iter().enumerate() {
            card = card.child(self.render_row(row, ix + 1 == count, &theme, cx));
        }
        div()
            .id("help-window")
            .key_context("Help")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::close))
            .size_full()
            .overflow_y_scroll()
            .bg(theme.bg)
            .font_family(theme.ui_font.clone())
            .text_color(theme.text)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .px(px(40.))
                    .pt(px(48.))
                    .pb(px(40.))
                    .child(logo(104.))
                    .child(
                        div()
                            .mt(px(20.))
                            .text_size(px(30.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text)
                            .child("Silica"),
                    )
                    .child(
                        div()
                            .mt(px(6.))
                            .text_size(px(16.))
                            .text_color(theme.text_muted)
                            .child(version),
                    )
                    .child(div().h(px(48.)))
                    .child(card),
            )
    }
}
