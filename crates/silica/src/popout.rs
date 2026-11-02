//! A popout window: a note's reading view in its own secondary window,
//! like Obsidian's "Open in new window".

use crate::reading::{build_with, ReadingView};
use crate::workspace::{VaultState, Workspace};
use gpui::{
    actions, prelude::*, px, size, AnyWindowHandle, App, Bounds, Context, Entity, FocusHandle,
    Focusable, SharedString, TitlebarOptions, WeakEntity, Window, WindowBackgroundAppearance,
    WindowBounds, WindowDecorations, WindowHandle, WindowOptions,
};

actions!(popout, [ClosePopout]);

/// Opens `path` in a popout window next to `main` (the workspace's own
/// window). Returns the window handle so the workspace can track it and
/// bring it forward if it is already open.
pub fn open(
    workspace: WeakEntity<Workspace>,
    path: String,
    state: std::sync::Arc<VaultState>,
    main: AnyWindowHandle,
    cx: &mut App,
) -> anyhow::Result<WindowHandle<PopoutWindow>> {
    let (w, h) = WINDOW_SIZE;
    let bounds = Bounds::centered(None, size(px(w), px(h)), cx);
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some(path.clone().into()),
                ..Default::default()
            }),
            app_id: Some("silica".into()),
            window_decorations: Some(WindowDecorations::Server),
            window_background: WindowBackgroundAppearance::Opaque,
            window_min_size: Some(size(px(480.), px(480.))),
            ..Default::default()
        },
        move |window, cx| {
            let view = cx.new(|cx| PopoutWindow {
                focus_handle: cx.focus_handle(),
                path: path.clone(),
                workspace,
                main,
                state: state.clone(),
                reading: cx.new(ReadingView::new),
            });
            // Build the model and set it on the reading view.
            let text = std::fs::read_to_string(state.vault().absolute(&path)).unwrap_or_default();
            let model = build_with(&path, &text, state.as_ref(), state.build_options());
            let reading = view.read(cx).reading.clone();
            reading.update(cx, |r, cx| {
                r.set_note(Some(path), model, cx);
            });
            let handle = view.read(cx).focus_handle.clone();
            window.focus(&handle, cx);
            view
        },
    )
}

/// The popout window's root view: a reading view showing one note.
pub struct PopoutWindow {
    focus_handle: FocusHandle,
    path: String,
    workspace: WeakEntity<Workspace>,
    main: AnyWindowHandle,
    state: std::sync::Arc<VaultState>,
    reading: Entity<ReadingView>,
}

impl Focusable for PopoutWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for PopoutWindow {
    fn render(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        self.reading.clone()
    }
}

/// Window size: roomy enough for a note without a scrollbar at default size.
const WINDOW_SIZE: (f32, f32) = (1000., 700.);
