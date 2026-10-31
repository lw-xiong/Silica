//! The element that draws a `.base`, and the state behind it.
//!
//! This module is the shell: it holds the parsed file, the view that is
//! showing, the table that view resolved to, and the writes the view makes.
//! The drawing itself belongs to a layout —
//! [`table`](crate::bases::table), [`cards`](crate::bases::cards),
//! [`list`](crate::bases::list) and [`kanban`](crate::bases::kanban), the
//! four layouts Obsidian registers — each of which adds its own methods to
//! [`BasesView`] and keeps its own state in one of its fields. The toolbar
//! above them is [`toolbar`](crate::bases::toolbar) and editing a cell is
//! [`edit`](crate::bases::edit).
//!
//! What the shell itself draws is the two states that are not a layout, both
//! observed (`tools/obsidian-probe/results/2026-10-25-bases.json`; the prose
//! is in
//! [[Specification#Observed Bases table view, Obsidian 1.14.5 (installer build 1.13.7)]]):
//!
//! - a **parse error** as `.bases-error`, two paragraphs: the banner
//!   [`PARSE_ERROR_BANNER`] and then the message, with nothing joining them;
//! - an **unknown view type** as the same block with one paragraph,
//!   "Unknown view type: x", and no rows.
//!
//! **Writing.** Obsidian writes a base on a toolbar change and canonicalizes
//! it on close, and never writes one that failed to parse. So does this: a
//! change goes through [`BasesView::edit_view`] or
//! [`BasesView::edit_base`], which re-resolve the table from the parsed file
//! and emit [`BasesEvent::Edited`]; the workspace's save path asks for
//! [`BasesView::file_contents`] — `Base::to_yaml`, Obsidian's own bytes — and
//! hands the written text back to [`BasesView::mark_saved`] so the watcher
//! can tell the view's own write from somebody else's.

use crate::bases::rows;
use crate::theme::{Theme, FONT_UI_SMALL};
use crate::workspace::VaultState;
use gpui::{
    div, prelude::*, px, AnyElement, Context, EventEmitter, FocusHandle, Focusable, Render,
    SharedString, Window,
};
use serde_json::{json, Value as Json};
use silica_bases::{Layout, Table};
use silica_formats::base::{Base, View, PARSE_ERROR_BANNER};
use std::sync::Arc;

/// What the view asks the workspace to do; it owns no vault of its own.
pub enum BasesEvent {
    /// A row was activated: open this file.
    Open(String),
    /// The base changed and is dirty: the autosave timer starts, as the
    /// canvas's own edits do.
    Edited,
    /// Something the view has to say, as a Notice — "Copied to your
    /// clipboard" and the rest.
    Report(String),
    /// A cell was typed into: set `key` to `value` in the **note** the row
    /// stands for.
    ///
    /// The view does not write it. A base's row is another file, and no view
    /// in this shell writes a file it does not hold — the workspace does,
    /// because only it can see whether that note is open in a buffer with
    /// unsaved edits, which a write behind its back would throw away.
    SetProperty {
        path: String,
        key: String,
        value: String,
    },
}

pub struct BasesView {
    focus: FocusHandle,
    path: Option<String>,
    /// The bytes the view last read, so the workspace can tell its own write
    /// coming back through the watcher from somebody else's.
    source: String,
    /// Which of the base's views is showing, by index into `base.views`.
    view_ix: usize,
    /// The file as it parsed, which is both what the layouts read their own
    /// keys off and what a write serializes.
    base: Option<Base>,
    table: Option<Table>,
    /// The showing view's layout and the keys that layout reads.
    layout: Layout,
    /// A file that does not parse shows this instead of a table, under
    /// [`silica_formats::base::PARSE_ERROR_BANNER`].
    error: Option<String>,
    state: Option<Arc<VaultState>>,
    /// Set when the view changed the base and the bytes on disk are older.
    dirty: bool,
    pub(super) table_state: super::table::TableState,
    pub(super) cards: super::cards::CardsState,
    pub(super) list: super::list::ListLayoutState,
    pub(super) kanban: super::kanban::KanbanState,
    pub(super) toolbar: super::toolbar::ToolbarState,
    pub(super) editing: super::edit::EditState,
}

impl BasesView {
    pub fn new(cx: &mut Context<Self>) -> BasesView {
        BasesView {
            focus: cx.focus_handle(),
            path: None,
            source: String::new(),
            view_ix: 0,
            base: None,
            table: None,
            layout: Layout::default(),
            error: None,
            state: None,
            dirty: false,
            table_state: super::table::TableState::default(),
            cards: super::cards::CardsState::default(),
            list: super::list::ListLayoutState::default(),
            kanban: super::kanban::KanbanState::default(),
            toolbar: super::toolbar::ToolbarState::default(),
            editing: super::edit::EditState::default(),
        }
    }

    /// Shows `path` with the bytes just read from it. An empty file is a base
    /// with one default view, which is what `bases:new-file` writes.
    pub fn load(&mut self, path: &str, text: &str, state: Arc<VaultState>, cx: &mut Context<Self>) {
        self.path = Some(path.to_string());
        self.source = text.to_string();
        self.state = Some(state);
        // A different file starts on its first view, as opening one does,
        // and carries none of the last one's unsaved state.
        self.view_ix = 0;
        self.dirty = false;
        self.toolbar = super::toolbar::ToolbarState::default();
        self.editing = super::edit::EditState::default();
        self.resolve(cx);
    }

    /// The file changed underneath the view.
    ///
    /// Obsidian reloads the live query from the new bytes and keeps the view
    /// the leaf was on (the external rewrite of the fixture came back with its
    /// renamed view already in the query), so the index is kept here too and
    /// only clamped if the rewrite dropped views. An outside edit is never
    /// clobbered: the view takes the new bytes and its own unsaved change
    /// goes with them.
    pub fn reload(&mut self, text: &str, cx: &mut Context<Self>) {
        self.source = text.to_string();
        self.dirty = false;
        self.editing = super::edit::EditState::default();
        self.resolve(cx);
    }

    /// Lets go of the file, as the canvas does when another kind opens.
    pub fn release(&mut self, cx: &mut Context<Self>) {
        self.path = None;
        self.base = None;
        self.table = None;
        self.layout = Layout::default();
        self.error = None;
        self.source.clear();
        self.state = None;
        self.dirty = false;
        self.toolbar = super::toolbar::ToolbarState::default();
        self.editing = super::edit::EditState::default();
        self.rebuild();
        cx.notify();
    }

    /// The vault changed under an open base: a note it queries was added,
    /// edited or removed. The bytes of the `.base` itself are unchanged, so
    /// the query is the same one and only its rows are resolved again.
    ///
    /// A base is a view over the whole vault rather than over one file, which
    /// is what makes this necessary at all: the canvas rebuilds a card when
    /// the file behind it changes, but every change is potentially a change
    /// to a base's table.
    pub fn invalidate(&mut self, state: Arc<VaultState>, cx: &mut Context<Self>) {
        if self.path.is_none() {
            return;
        }
        self.state = Some(state);
        // A base the view has edited but not yet saved must not be re-parsed
        // from the bytes on disk: the edit is in `base`, not in `source`.
        if self.dirty {
            self.resolve_table();
            self.rebuild();
            cx.notify();
            return;
        }
        self.resolve(cx);
    }

    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    /// Whether the bytes on disk are the ones the view read.
    pub fn matches_file(&self, text: &str) -> bool {
        self.source == text
    }

    /// Whether the view holds a change the disk does not.
    pub fn is_dirty(&self) -> bool {
        self.dirty && self.base.is_some()
    }

    /// The bytes a save writes: the parsed base in Obsidian's canonical form.
    /// `None` when there is nothing to write, which includes every file that
    /// failed to parse — a broken base is never written.
    pub fn file_contents(&self) -> Option<String> {
        self.base.as_ref().map(Base::to_yaml)
    }

    /// The save landed: these are now the bytes on disk, so the watcher event
    /// that follows is the view's own write rather than somebody else's.
    pub fn mark_saved(&mut self, text: String) {
        self.source = text;
        self.dirty = false;
    }

    /// Parses [`BasesView::source`] and resolves the showing view against the
    /// vault. A file that does not parse leaves no table at all, which is what
    /// Obsidian does: the query is the error object, and nothing is drawn but
    /// the banner.
    fn resolve(&mut self, cx: &mut Context<Self>) {
        let resolved = rows::resolve(self.state.as_deref(), self.path.as_deref(), &self.source, 0);
        self.base = resolved.base;
        self.error = resolved.error;
        // A base with no views has one default view, so index 0 is always
        // valid; anything past the end falls back to it.
        if self.view_ix > 0 && self.view_ix >= self.base.as_ref().map_or(0, |b| b.views.len()) {
            self.view_ix = 0;
        }
        self.resolve_table();
        self.rebuild();
        cx.notify();
    }

    /// Resolves the showing view against the vault from the base already
    /// parsed, which is the path an edit takes: the file on disk is the old
    /// one and must not be re-read.
    fn resolve_table(&mut self) {
        let Some(base) = self.base.as_ref() else {
            self.table = None;
            self.layout = Layout::default();
            return;
        };
        let env = match (self.state.as_ref(), self.path.as_deref()) {
            (Some(state), Some(path)) => rows::env_for(state, path),
            _ => silica_bases::Env {
                files: Vec::new(),
                base_file: None,
                now: chrono::Local::now().naive_local(),
            },
        };
        self.table = Some(silica_bases::table::build(base, self.view_ix, &env));
        self.layout = match base.views.get(self.view_ix) {
            Some(view) => Layout::of(view),
            // An empty base is one default table of the whole vault.
            None => Layout::default(),
        };
    }

    /// Rebuilds whichever layout is showing. Only one runs: a kanban's
    /// columns cost real work over a 20 000-note vault, and nothing but the
    /// showing layout is drawn.
    fn rebuild(&mut self) {
        match self.layout {
            Layout::Table(_) | Layout::Unknown(_) => self.rebuild_table(),
            Layout::Cards(_) => self.rebuild_cards(),
            Layout::List(_) => self.rebuild_list(),
            Layout::Kanban(_) => self.rebuild_kanban(),
        }
    }

    // ----- what the layouts read ---------------------------------------------

    /// The view whose rows are showing, when the file parsed and has one.
    pub(super) fn view(&self) -> Option<&View> {
        self.base.as_ref()?.views.get(self.view_ix)
    }

    /// The showing view's index into `base.views`.
    pub(super) fn view_ix(&self) -> usize {
        self.view_ix
    }

    pub(super) fn table(&self) -> Option<&Table> {
        self.table.as_ref()
    }

    pub(super) fn base(&self) -> Option<&Base> {
        self.base.as_ref()
    }

    pub(super) fn layout(&self) -> &Layout {
        &self.layout
    }

    pub(super) fn vault(&self) -> Option<&Arc<VaultState>> {
        self.state.as_ref()
    }
    // ----- what the layouts do -----------------------------------------------

    /// Opens the file a row names, which is the workspace's to do.
    pub(super) fn open(&mut self, path: String, cx: &mut Context<Self>) {
        cx.emit(BasesEvent::Open(path));
    }

    /// Says something as a Notice.
    pub(super) fn report(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        cx.emit(BasesEvent::Report(message.into()));
    }

    /// Asks the workspace to set a property of the note a row stands for,
    /// which is what a cell typed into does. See
    /// [`BasesEvent::SetProperty`] for why the view does not write it
    /// itself.
    pub(super) fn set_property(
        &mut self,
        path: &str,
        key: &str,
        value: &str,
        cx: &mut Context<Self>,
    ) {
        cx.emit(BasesEvent::SetProperty {
            path: path.to_string(),
            key: key.to_string(),
            value: value.to_string(),
        });
    }

    /// Shows another of the base's views, as the views menu and
    /// `bases:change-view` do. Out of range is ignored rather than clamped:
    /// nothing should silently show a view the caller did not name.
    pub(super) fn show_view(&mut self, ix: usize, cx: &mut Context<Self>) {
        let count = self.base.as_ref().map_or(0, |b| b.views.len());
        if ix >= count.max(1) {
            return;
        }
        self.view_ix = ix;
        self.editing = super::edit::EditState::default();
        self.resolve_table();
        self.rebuild();
        cx.notify();
    }

    /// Changes the showing view and writes the base.
    ///
    /// This is the one way a layout or the toolbar changes the file: the
    /// closure edits the [`View`], the table is resolved again from the
    /// parsed base — never from the bytes on disk, which are still the old
    /// ones — and the workspace is told to save.
    pub(super) fn edit_view<F>(&mut self, edit: F, cx: &mut Context<Self>)
    where
        F: FnOnce(&mut View),
    {
        let ix = self.view_ix;
        self.edit_base(
            move |base| {
                // An empty base has no view to edit: the default view is
                // materialized so the change has somewhere to land, which is
                // also what Obsidian's own close writes.
                if base.views.is_empty() {
                    base.views.push(View {
                        kind: silica_formats::base::DEFAULT_VIEW_TYPE.to_string(),
                        name: silica_formats::base::DEFAULT_VIEW_NAME.to_string(),
                        ..View::default()
                    });
                }
                if let Some(view) = base.views.get_mut(ix) {
                    edit(view);
                }
            },
            cx,
        );
    }

    /// Changes the base itself — its own filters, formulas, properties or the
    /// list of views — and writes it.
    pub(super) fn edit_base<F>(&mut self, edit: F, cx: &mut Context<Self>)
    where
        F: FnOnce(&mut Base),
    {
        let Some(base) = self.base.as_mut() else {
            return;
        };
        edit(base);
        self.dirty = true;
        self.resolve_table();
        self.rebuild();
        cx.emit(BasesEvent::Edited);
        cx.notify();
    }

    /// What the control socket reports under `bases`.
    pub fn state_json(&self) -> Json {
        let columns: Vec<Json> = self
            .table
            .iter()
            .flat_map(|t| t.columns.iter())
            .map(|c| json!({ "id": c.id, "display": c.display }))
            .collect();
        let rows: Vec<Json> = self
            .table
            .iter()
            .flat_map(|t| t.rows.iter())
            .take(STATE_ROWS)
            .map(|r| {
                json!({
                    "file": r.file.path,
                    "cells": r.cells.iter().map(super::cells::cell_text).collect::<Vec<_>>(),
                })
            })
            .collect();
        let groups: Vec<Json> = self
            .table
            .iter()
            .flat_map(|t| t.groups.iter())
            .map(|g| json!({ "key": g.key, "rows": g.rows.len() }))
            .collect();
        let views: Vec<Json> = self
            .base
            .iter()
            .flat_map(|b| b.views.iter())
            .map(|v| json!({ "name": v.name, "type": v.kind }))
            .collect();
        json!({
            "path": self.path,
            "error": self.error,
            "view": self.view_ix,
            "view_name": self.view().map(|v| v.name.clone()),
            "views": views,
            "layout": self.layout.kind(),
            "layout_state": self.layout_state_json(),
            "unknown_kind": self.table.as_ref().and_then(|t| t.unknown_kind.clone()),
            "columns": columns,
            "rows": rows,
            "rows_total": self.table.as_ref().map_or(0, |t| t.rows.len()),
            "limited": self.table.as_ref().is_some_and(|t| t.limited),
            "groups": groups,
            "dirty": self.dirty,
            "toolbar": self.toolbar.state_json(),
            "editing": self.editing.state_json(),
        })
    }

    /// What the showing layout reports for itself.
    fn layout_state_json(&self) -> Json {
        match self.layout {
            Layout::Table(_) | Layout::Unknown(_) => self.table_state.state_json(),
            Layout::Cards(_) => self.cards.state_json(),
            Layout::List(_) => self.list.state_json(),
            Layout::Kanban(_) => self.kanban.state_json(),
        }
    }

    // ----- rendering ----------------------------------------------------------

    /// `.bases-error`: one paragraph per message, in the view's body.
    ///
    /// A parse failure draws two — [`PARSE_ERROR_BANNER`] and then the
    /// message, with nothing joining them — and an unknown view type draws one.
    /// The message is kept line for line in the mono family because the YAML
    /// library's own text carries a caret under the offending column, which
    /// only lines up in a fixed pitch. **Unprobed**: the block's own type and
    /// colour were never read off the running app.
    fn render_error(&self, banner: bool, message: &str, theme: &Theme) -> AnyElement {
        let mut block = div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .p(px(24.))
            .text_size(px(theme.base_font_size))
            .text_color(theme.text);
        if banner {
            block = block.child(SharedString::from(PARSE_ERROR_BANNER));
        }
        block
            .child(
                div()
                    .flex()
                    .flex_col()
                    .font_family(theme.mono_font.clone())
                    .text_size(px(FONT_UI_SMALL))
                    .text_color(theme.text_muted)
                    .children(message.lines().map(|line| {
                        // An empty line still needs a line box, or the caret
                        // under the snippet climbs onto the text above it.
                        div().child(SharedString::from(if line.is_empty() {
                            " ".to_string()
                        } else {
                            line.to_string()
                        }))
                    })),
            )
            .into_any_element()
    }

    /// A view that resolved to no rows at all, which every layout draws
    /// through here.
    ///
    /// The **wording is observed**: `BadFilter.base` — whose only filter is
    /// `density >>> 1` — "reports 0 results" with its header row still drawn
    /// and the error banner hidden, so a view that empties says how many
    /// results it has rather than that something went wrong. **Unprobed**:
    /// where that count sits and what it looks like. The box is the shell's
    /// own `pane-empty` — 13 px faint, centred, padded `24px 16px` — which is
    /// what the outline pane draws for the same situation.
    pub(super) fn render_empty(&self, theme: &Theme) -> AnyElement {
        div()
            .flex_1()
            .w_full()
            .flex()
            .items_start()
            .justify_center()
            .py(px(24.))
            .px(px(16.))
            .text_size(px(FONT_UI_SMALL))
            .text_color(theme.text_faint)
            .child(SharedString::from("0 results"))
            .into_any_element()
    }
}

/// How many rows [`BasesView::state_json`] lists before it stops; the whole
/// count is always there as `rows_total`.
const STATE_ROWS: usize = 100;

impl EventEmitter<BasesEvent> for BasesView {}

impl Focusable for BasesView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for BasesView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let body: Vec<AnyElement> = if let Some(message) = self.error.clone() {
            vec![self.render_error(true, &message, &theme)]
        } else if let Some(kind) = self.table.as_ref().and_then(|t| t.unknown_kind.clone()) {
            // The file parsed; only the layout is one this build cannot draw.
            vec![self.render_error(false, &format!("Unknown view type: {kind}"), &theme)]
        } else if self.table.is_some() {
            let mut body = Vec::new();
            body.extend(self.render_toolbar(&theme, cx));
            body.extend(match self.layout.clone() {
                Layout::Cards(_) => self.render_cards(&theme, cx),
                Layout::List(_) => self.render_list(&theme, cx),
                Layout::Kanban(_) => self.render_kanban(&theme, cx),
                // An unknown type never reaches here; it is the branch above.
                Layout::Table(_) | Layout::Unknown(_) => self.render_table(&theme, cx),
            });
            body
        } else {
            Vec::new()
        };
        div()
            .track_focus(&self.focus)
            .key_context("Bases")
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.bg)
            .text_color(theme.text)
            .children(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_base_that_does_not_parse_has_a_message_and_no_table() {
        // The parse itself is `silica-formats`'s; what matters here is that
        // the shell keeps the message and draws no rows for it.
        let resolved = rows::resolve(None, None, "views:\n  - type: table\n", 0);
        assert!(resolved.error.is_some(), "a view with no name is refused");
        assert!(resolved.base.is_none());
    }

    #[test]
    fn an_empty_file_is_one_default_table_of_the_whole_vault() {
        let resolved = rows::resolve(None, None, "", 0);
        assert!(resolved.error.is_none());
        let base = resolved.base.expect("an empty file parses");
        assert!(base.views.is_empty(), "the default view is not in the file");
        let table = resolved.table.expect("and still resolves to a table");
        assert_eq!(table.unknown_kind, None);
    }

    #[test]
    fn an_unknown_type_resolves_to_no_rows_and_names_itself() {
        let resolved = rows::resolve(None, None, "views:\n  - type: nonsense\n    name: V\n", 0);
        let table = resolved.table.expect("the file parses");
        assert_eq!(table.unknown_kind.as_deref(), Some("nonsense"));
        assert!(table.rows.is_empty());
    }
}
