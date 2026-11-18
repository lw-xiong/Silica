//! Mouse interaction: click to place the cursor, double/triple click
//! selection, drag selection, alt-click multi-cursor, link hover and
//! Ctrl+hover page preview, wheel scrolling, and the right-click context
//! menu — the methods of the split-out mouse section, unchanged apart from
//! living in their own module. Split from `editor.rs` per the audit's
//! module-size finding.

use std::ops::Range;
use std::sync::Arc;

use gpui::{point, px, Context, MouseDownEvent, Pixels, Point, Window};

use crate::reading::model::LinkTarget;
use crate::workspace::VaultState;
use silica_formats::properties as props;
use silica_suggest::blocks::{self, IdInsertion, Placement};
use silica_suggest::{self as suggest, Accept, BlockId, Suggestion};

use super::{newline_of, CellEdit, ContextMenu, Editor, EditorEvent, Hit, SuggestState};
use crate::preview;
use crate::properties::{self, Field, Part, PillStep, Target};
use crate::reading::view::{PropertyAction, TableAction};
use crate::suggest::Anchor;
use silica_markdown::highlight::HighlightKind;
use silica_text::table;
use silica_text::{Cursors, EditKind, Sel};

impl Editor {
    /// The visible line under a window point, shaped as painted, and the
    /// display byte nearest the point.
    /// Reopens, refreshes or closes the autocompletion popup for the text
    /// before the cursor.
    ///
    /// `opening` is set after an edit. A bare cursor move only refreshes a
    /// popup that is already open, so clicking into the middle of a link
    /// that is already written does not pop one up.
    pub(crate) fn update_suggest(&mut self, opening: bool, cx: &mut Context<Self>) {
        if !opening && self.suggest.is_none() {
            return;
        }
        // * One query, one cursor: a suggestion inserted at several at once
        //   would be several different links.
        if self.cursor_count() > 1 || self.property_field.is_some() {
            self.close_suggest(cx);
            return;
        }
        let Some(state) = self.suggest_vault.clone() else {
            self.close_suggest(cx);
            return;
        };
        let cursor = self.cursor();
        let line = self.buf.line_text(cursor.line).to_string();
        let Some(t) = suggest::trigger(&line, cursor.col) else {
            self.close_suggest(cx);
            return;
        };
        let items = self.offer(&state, &t);
        if items.is_empty() {
            self.close_suggest(cx);
            return;
        }
        self.suggest = Some(SuggestState {
            line: cursor.line,
            range: t.range.clone(),
            items,
            selected: 0,
        });
        cx.notify();
    }

    /// The rows the vault offers for `t`, read under its locks.
    pub(crate) fn offer(
        &self,
        state: &Arc<VaultState>,
        t: &suggest::Triggered<'_>,
    ) -> Vec<Suggestion> {
        let vault = state.vault();
        let cache = state.cache();
        let show_unsupported = state.app().show_unsupported_files.unwrap_or(false);
        // * Only a `#^` query asks for a note's text, so this costs nothing
        //   on the common `[[` keystroke.
        let texts = |path: &str| state.texts().get(path).map(|t| t.to_string());
        suggest::suggestions(
            t,
            &suggest::Vault {
                files: vault.files(),
                cache: &cache,
                source: self.name.as_ref(),
                show_unsupported,
                text: &texts,
            },
        )
    }

    pub(crate) fn close_suggest(&mut self, cx: &mut Context<Self>) {
        if self.suggest.take().is_some() {
            cx.notify();
        }
    }

    /// Moves the popup's selection, if it is open. Up, down, the page keys
    /// and Home and End all belong to the popup while it is, which is what
    /// Obsidian's popup binds.
    pub(crate) fn suggest_step(&mut self, delta: isize, cx: &mut Context<Self>) -> bool {
        let Some(s) = self.suggest.as_mut() else {
            return false;
        };
        match delta {
            isize::MIN => s.select_first(),
            isize::MAX => s.select_last(),
            delta => s.step(delta),
        }
        cx.notify();
        true
    }

    /// Accepts the selected row, if the popup is open.
    pub(crate) fn accept_suggestion(
        &mut self,
        accept: Accept,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(ix) = self.suggest.as_ref().map(|s| s.selected) else {
            return false;
        };
        self.accept_suggestion_at(ix, accept, window, cx)
    }

    /// Writes the row at `ix` into the document and puts the cursor where
    /// the accept key leaves it. The trigger is read from the line again so
    /// the text that is replaced is the text that is there now.
    pub(crate) fn accept_suggestion_at(
        &mut self,
        ix: usize,
        accept: Accept,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(state) = self.suggest_vault.clone() else {
            return false;
        };
        let Some((item, at, end)) = self.suggest.as_ref().and_then(|s| {
            let item = s.items.get(ix).map(|row| row.item.clone())?;
            Some((item, s.line, s.range.end))
        }) else {
            return false;
        };
        // * A block with no id of its own cannot be linked until one is
        //   written. The id goes in first, into this note or into the other
        //   one, and the link is only written if that succeeded.
        let (item, own_note) = match item {
            suggest::Item::Block {
                path,
                id: BlockId::New { end, placement },
            } => match self.write_block_id(&state, &path, end, placement, cx) {
                Some((id, own)) => (
                    suggest::Item::Block {
                        path,
                        id: BlockId::Existing(id),
                    },
                    own,
                ),
                None => {
                    self.close_suggest(cx);
                    return false;
                }
            },
            other => (other, None),
        };
        let line_start = self.buf.line_range(at).start;
        let line = self.buf.line_text(at).to_string();
        let Some(t) = suggest::trigger(&line, end) else {
            self.close_suggest(cx);
            return false;
        };
        let ins = {
            let vault = state.vault();
            let cache = state.cache();
            let show_unsupported = state.app().show_unsupported_files.unwrap_or(false);
            let texts = |path: &str| state.texts().get(path).map(|t| t.to_string());
            suggest::insert(
                &t,
                &item,
                accept,
                &suggest::Vault {
                    files: vault.files(),
                    cache: &cache,
                    source: self.name.as_ref(),
                    show_unsupported,
                    text: &texts,
                },
                &self.link_options,
            )
        };
        let range = line_start + t.range.start..line_start + t.range.end + ins.consume;
        let mut cursor = range.start + ins.cursor;
        self.suggest = None;
        // * An id written into this very note is part of the same edit, so
        //   one undo takes back the whole accept.
        let mut edits = vec![(range.clone(), ins.text)];
        if let Some(id_ins) = own_note {
            if id_ins.range.start <= range.start {
                let grew = id_ins.text.len() as isize - id_ins.range.len() as isize;
                cursor = (cursor as isize + grew).max(0) as usize;
            }
            edits.push((id_ins.range, id_ins.text));
        }
        // * One undo step, and the cursor where the insertion asks for it:
        //   after the link for Enter, inside the brackets for Tab, where
        //   `after_edit` opens the popup again on what is left to type.
        self.edit_many(edits, cursor, None, window, cx);
        true
    }

    /// Writes an id onto the block a row points at, so the row can be
    /// linked to.
    ///
    /// Returns the id, and the edit to fold into this note's own when the
    /// block is in the note being edited — that case has to go through the
    /// buffer, or the next save would write the id straight back out.
    /// `None` when nothing could be written, and the accept is abandoned.
    pub(crate) fn write_block_id(
        &mut self,
        state: &Arc<VaultState>,
        path: &str,
        end: usize,
        placement: Placement,
        cx: &mut Context<Self>,
    ) -> Option<(String, Option<IdInsertion>)> {
        let target = if path.is_empty() {
            self.name.to_string()
        } else {
            path.to_string()
        };
        let own = target == self.name.as_ref();
        // * The note being edited is planned against the buffer, which is
        //   ahead of the disk; another note against its own bytes.
        let text = if own {
            self.buf.text().to_string()
        } else {
            state.texts().get(&target)?.to_string()
        };
        let id = {
            let cache = state.cache();
            let taken = cache.file(&target).map(|m| m.blocks.clone())?;
            blocks::new_id(self.id_seed(), &taken)
        };
        let nl = if self.buf.crlf() && own {
            "\r\n"
        } else {
            newline_of(&text)
        };
        // ! A block an id cannot be written onto is refused rather than
        //   written to, so a link Silica writes always resolves.
        let Some(insertion) = blocks::plan_id(&text, end, placement, &id, nl) else {
            cx.emit(EditorEvent::Reported(
                format!("No id can be written onto that block in {target}").into(),
            ));
            return None;
        };
        if own {
            return Some((id, Some(insertion)));
        }
        match state.write_block_id(&target, &text, &insertion) {
            Ok(()) => Some((id, None)),
            Err(err) => {
                cx.emit(EditorEvent::Reported(err.into()));
                None
            }
        }
    }

    /// The heading the cursor's line is, as `#Heading` — the subpath
    /// "Bookmark heading under cursor..." records. `None` off a heading.
    pub fn heading_subpath_at_cursor(&self) -> Option<String> {
        let line = self.cursor().line as u32;
        let meta = silica_cache::index_markdown(self.buf.text());
        meta.headings
            .iter()
            .find(|h| h.position.start.line == line)
            .map(|h| format!("#{}", h.heading.trim()))
    }

    /// The id of the block under the cursor, written into the note first
    /// when it has none — what "Bookmark block under cursor..." does. The
    /// buffer, not the disk, is what the cursor is in, so the block is
    /// found in the buffer's own index and the id goes in as one edit the
    /// autosave writes. `None` on a block that takes no id (a list as a
    /// whole, a thematic break, the frontmatter), which is reported.
    pub fn block_id_at_cursor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let line = self.cursor().line as u32;
        let text = self.buf.text().to_string();
        let meta = silica_cache::index_markdown(&text);
        let section = meta
            .sections
            .iter()
            .rfind(|s| s.position.start.line <= line && line <= s.position.end.line)?;
        if let Some(id) = &section.id {
            return Some(id.clone());
        }
        let Some(placement) = Placement::of(section.kind) else {
            cx.emit(EditorEvent::Reported(
                "That block cannot take a block id".into(),
            ));
            return None;
        };
        let id = blocks::new_id(self.id_seed(), &meta.blocks);
        let nl = if self.buf.crlf() {
            "\r\n"
        } else {
            newline_of(&text)
        };
        let Some(insertion) =
            blocks::plan_id(&text, section.position.end.offset, placement, &id, nl)
        else {
            cx.emit(EditorEvent::Reported(
                "No id can be written onto that block".into(),
            ));
            return None;
        };
        let cursor = self.buf.head;
        self.edit_many(
            vec![(insertion.range.clone(), insertion.text.clone())],
            cursor,
            None,
            window,
            cx,
        );
        Some(id)
    }

    /// A seed for a fresh block id. The clock is the only randomness the
    /// workspace has, and an id only has to be unique within its note.
    pub(crate) fn id_seed(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
            ^ (self.buf.head as u64).wrapping_mul(0x9e37_79b9)
    }

    /// Where the popup is drawn: under the start of the query, in the
    /// editor's own coordinates.
    pub(crate) fn suggest_anchor(&self, state: &SuggestState) -> Option<Anchor> {
        let shaped = self.layout.cursor_line.clone()?;
        let (row, x) = shaped.row_and_x(state.range.start.min(state.range.end));
        let rh = shaped.row_height;
        let origin = self.layout.bounds.origin;
        let top = self.layout.cursor_top - origin.y + rh * row as f32;
        Some(Anchor {
            x: self.layout.content_x - origin.x + x,
            top,
            bottom: top + rh,
            width: self.layout.bounds.size.width,
            height: self.layout.bounds.size.height,
        })
    }

        /// `editor:context-menu`: the same menu a right-click shows, opened at
    /// the caret — the anchor math is `suggest_anchor`'s, with the caret's
    /// offset as the target.
    pub(crate) fn open_context_menu_at_caret(&mut self, cx: &mut Context<Self>) {
        let offset = self.selection().end;
        let Some(shaped) = self.layout.cursor_line.clone() else {
            return;
        };
        let (row, x) = shaped.row_and_x(offset);
        let origin = self.layout.bounds.origin;
        let top = self.layout.cursor_top - origin.y + shaped.row_height * row as f32;
        let position = point(
            self.layout.content_x - origin.x + x,
            top + shaped.row_height,
        );
        self.context_menu = Some(ContextMenu {
            position,
            target: Some(offset),
        });
        cx.notify();
    }

/// Resolves a window point against the last frame's layout.
    pub(crate) fn display_hit(&self, window: &mut Window, position: Point<Pixels>) -> Option<Hit> {
        let layout = &self.layout;
        if layout.line_height <= px(0.) || layout.visible.is_empty() {
            return None;
        }
        let y = position.y - layout.bounds.origin.y;
        let x = position.x - layout.content_x;
        // * A point above the first line or below the last still names a row,
        //   so the caret has somewhere to go, but it is on no line of text:
        //   `on_line` carries that down to the glyph.
        let (vl, row, on_line) = match layout
            .visible
            .iter()
            .find(|v| y >= v.top && y < v.top + v.height())
        {
            Some(v) => (*v, ((y - v.top) / v.row_height).floor() as usize, true),
            None if y < layout.visible[0].top => (layout.visible[0], 0, false),
            None => {
                let last = *layout.visible.last()?;
                (last, last.rows - 1, false)
            }
        };
        let shaped = self.shape(window, vl.line)?;
        let display = shaped.display_index_at(x, row);
        let glyph = if on_line {
            shaped.glyph_index_at(x, row)
        } else {
            None
        };
        Some(Hit {
            line: vl.line,
            shaped,
            display,
            glyph,
        })
    }

    /// The buffer offset under a window point, using the last frame's layout.
    pub(crate) fn offset_at_point(
        &self,
        window: &mut Window,
        position: Point<Pixels>,
    ) -> Option<usize> {
        let hit = self.display_hit(window, position)?;
        let col = self
            .buf
            .clamp_col(hit.line, hit.shaped.to_source(hit.display));
        Some(self.buf.line_starts()[hit.line] + col)
    }

    /// The link under source byte `col` of `line`. With `rendered_only`,
    /// only a link Live Preview shows rendered counts (a revealed one is
    /// being edited).
    pub(crate) fn link_at(
        &self,
        line: usize,
        col: usize,
        rendered_only: bool,
    ) -> Option<LinkTarget> {
        let spans = self.highlights.get(line)?;
        let touch = self.touches_on_line(line);
        let text = self.buf.line_text(line);
        for s in spans {
            if !(s.range.start <= col && col < s.range.end) {
                continue;
            }
            let revealed = preview::touches(&touch, &s.range);
            if rendered_only && revealed {
                continue;
            }
            let t = text.get(s.range.clone())?;
            let target = match s.kind {
                HighlightKind::Wikilink | HighlightKind::Embed => {
                    let inner = t
                        .trim_start_matches('!')
                        .trim_start_matches("[[")
                        .trim_end_matches("]]");
                    let body = inner.split('|').next().unwrap_or(inner);
                    LinkTarget::Internal(body.to_string())
                }
                HighlightKind::Link => {
                    // * `[text](dest "title")`; reference links are not followed.
                    let Some(open) = t.rfind("](") else {
                        continue;
                    };
                    let dest = t[open + 2..].trim_end_matches(')');
                    let dest = dest.split_whitespace().next().unwrap_or(dest);
                    let dest = dest.trim_matches(|c| c == '<' || c == '>');
                    if silica_markdown::html::is_vault_destination(dest) {
                        LinkTarget::Internal(
                            silica_markdown::html::percent_decode(dest).into_owned(),
                        )
                    } else {
                        LinkTarget::External(dest.to_string())
                    }
                }
                HighlightKind::Url => {
                    LinkTarget::External(t.trim_matches(|c| c == '<' || c == '>').to_string())
                }
                HighlightKind::Tag => LinkTarget::Tag(t.trim_start_matches('#').to_string()),
                _ => continue,
            };
            return Some(target);
        }
        None
    }

    /// Flips the task marker whose `[` is the byte at `col` on `line`:
    /// `[ ]` becomes `[x]` and anything else becomes `[ ]`, as one undo
    /// step that leaves the selection where it was (the three bytes are
    /// replaced by three, so no offset moves).
    ///
    /// Both rendered views toggle through here — Live Preview from its own
    /// click, the reading view by way of the workspace — so a checkbox
    /// writes the same bytes whichever one was clicked. Returns false, and
    /// writes nothing, when those bytes are not a marker: a reading view
    /// built before the note changed can name a position that has moved.
    pub fn toggle_task_at(
        &mut self,
        line: usize,
        col: usize,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(&start) = self.buf.line_starts().get(line) else {
            return false;
        };
        let checked = match self.buf.line_text(line).as_bytes().get(col..col + 3) {
            Some([b'[', status, b']']) => *status != b' ',
            _ => return false,
        };
        let with = if checked { "[ ]" } else { "[x]" };
        let (head, anchor) = (self.buf.head, self.buf.anchor);
        if self.buf.edit_many(
            vec![(start + col..start + col + 3, with.to_string())],
            head,
            Some(anchor),
        ) {
            self.after_edit(window, cx, None);
        }
        true
    }

    /// Live Preview clicks on rendered constructs: a checkbox toggles, a
    /// link opens (Ctrl+click opens a link in Source mode too). Returns
    /// true when the click was consumed.
    pub(crate) fn click_rendered(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if event.click_count != 1 || event.modifiers.shift {
            return false;
        }
        let Some(hit) = self.display_hit(window, event.position) else {
            return false;
        };
        // * The left margin, the blank space past the end of a row, and the
        //   area above and below the document are on no construct: the click
        //   places the caret and nothing more. Only what was drawn under the
        //   pointer can be worked.
        let Some(d) = hit.glyph else {
            return false;
        };
        let (line, shaped) = (hit.line, &hit.shaped);
        if self.live_preview {
            if let Some((src, HighlightKind::Task)) = shaped.display.sub_at(d) {
                self.toggle_task_at(line, src.start, Some(window), cx);
                return true;
            }
        }
        if self.live_preview || event.modifiers.control {
            let col = shaped.to_source(d);
            if let Some(target) = self.link_at(line, col, !event.modifiers.control) {
                cx.emit(if event.modifiers.control {
                    EditorEvent::OpenLinkInNewTab(target)
                } else {
                    EditorEvent::OpenLink(target)
                });
                return true;
            }
        }
        false
    }

    // ------------------------------------------------------- properties

    /// The properties block of the note as it stands, or `None` when it has
    /// none.
    pub(crate) fn properties(&self) -> Option<props::Properties> {
        silica_cache::note_properties(self.buf.text())
    }

    /// The display rows of the block, with each key's assigned type.
    pub(crate) fn property_rows(&self) -> Vec<properties::Row> {
        let Some(block) = self.properties() else {
            return Vec::new();
        };
        let mut next_id = crate::properties::PROPERTY_ID_BASE;
        properties::rows(
            &block.entries,
            |key| props::type_of(key, &block.entries, &self.property_types),
            &mut next_id,
        )
    }

    pub(crate) fn property_type_of(&self, key: &str) -> props::PropertyType {
        match self.properties() {
            Some(block) => props::type_of(key, &block.entries, &self.property_types),
            None => props::PropertyType::Text,
        }
    }

    /// Plans `changes` against the block and applies them as one undo step,
    /// so a field commit is one `Ctrl+Z` away however many lines it moved.
    pub(crate) fn apply_property_changes(
        &mut self,
        changes: &[props::Change],
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        let Some(block) = self.properties() else {
            // * No block yet: a new property creates one above the body.
            if let Some(props::Change::Set { key, value }) = changes.first() {
                let nl = if self.buf.crlf() { "\r\n" } else { "\n" };
                let text = props::new_block(key, value, nl);
                // * The buffer holds no BOM (it travels in `self.buf.bom()` and is
                //   put back on save), so the block goes at offset zero.
                self.edit(0..0, &text, EditKind::Other, window, cx);
            }
            return;
        };
        let Some(edits) = props::plan(self.buf.text(), &block, changes) else {
            return;
        };
        let ranges: Vec<(Range<usize>, String)> = edits
            .into_iter()
            .map(|e| (e.start..e.end, e.text))
            .collect();
        // * Keep the cursor where it was, clamped: the block above it may have
        //   grown or shrunk, and the caret lives in a field, not the document.
        let head = self.buf.head.min(self.buf.len());
        let after = Cursors {
            sels: vec![Sel::at(head)],
            primary: 0,
        };
        self.buf.apply_edits(ranges, after, EditKind::Other);
        self.after_edit(window, cx, None);
    }

    /// `markdown:add-metadata-property`: opens an unnamed row at the end of
    /// the block. Obsidian writes nothing to the file until the row is named
    /// (confirmed with a probe: running the command left the note untouched).
    pub fn add_property_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.property_action(PropertyAction::AddRow, window, cx);
    }

    /// `markdown:clear-metadata-properties`: removes every property, and with
    /// it the fences, as Obsidian does (`---\na: 1\n---\nbody\n` leaves
    /// `body\n`).
    pub fn clear_properties(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.property_field = None;
        self.selected_pill = None;
        let Some(block) = self.properties() else {
            return;
        };
        let changes = properties::clear_all(&block.entries);
        if !changes.is_empty() {
            self.apply_property_changes(&changes, Some(window), cx);
        }
    }

    /// `editor:toggle-fold-properties`.
    pub fn toggle_properties_fold(&mut self, cx: &mut Context<Self>) {
        self.properties_folded = !self.properties_folded;
        cx.notify();
    }

    /// What the reader did to a property row.
    pub(crate) fn property_action(
        &mut self,
        action: PropertyAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // * A click is not a Backspace: whatever pill one selected, it loses
        //   the selection.
        self.selected_pill = None;
        match action {
            PropertyAction::Edit { target, part } => {
                self.property_menu = None;
                self.commit_property_field(Some(window), cx);
                // * A value opens through `value_field`, which lands in the
                //   empty input after the pills when the row holds a list:
                //   opening a list whole would commit it back through the
                //   joined-string round trip and split an item at its comma.
                let row = match (&target, part) {
                    (Target::Existing(key), Part::Value) => {
                        self.property_rows().into_iter().find(|r| &r.key == key)
                    }
                    _ => None,
                };
                self.property_field = Some(match row {
                    Some(row) => properties::value_field(&row),
                    None => {
                        let text = match (&target, part) {
                            (Target::Existing(key), Part::Key) => key.clone(),
                            _ => String::new(),
                        };
                        Field::new(target, part, text)
                    }
                });
                cx.notify();
            }
            PropertyAction::EditItem { key, index } => {
                self.property_menu = None;
                self.commit_property_field(Some(window), cx);
                // * The commit above may have rewritten the block, so the row
                //   the field is built from is read after it, not before.
                if let Some(row) = self.property_rows().into_iter().find(|r| r.key == key) {
                    self.property_field = Some(row.item_field(index.min(row.items.len())));
                }
                cx.notify();
            }
            PropertyAction::RemoveItem { key, index } => {
                self.property_field = None;
                if let Some(row) = self.property_rows().into_iter().find(|r| r.key == key) {
                    if let Some(change) = properties::remove_item(&row, index) {
                        self.apply_property_changes(&[change], Some(window), cx);
                    }
                }
            }
            PropertyAction::Remove { key } => {
                self.property_field = None;
                self.apply_property_changes(&[props::Change::Remove { key }], Some(window), cx);
            }
            PropertyAction::Retype { key, ty } => {
                self.property_menu = None;
                if let Some(row) = self.property_rows().into_iter().find(|r| r.key == key) {
                    let change = properties::retype(&row, ty);
                    self.apply_property_changes(&[change], Some(window), cx);
                }
                // * The assignment belongs in the vault's `types.json`, which
                //   the workspace owns; it writes the file and hands the new
                //   table back through `set_property_types`.
                self.property_types
                    .types
                    .insert(key.clone(), ty.as_str().to_string());
                cx.emit(EditorEvent::PropertyTypeAssigned { key, ty });
                cx.notify();
            }
            PropertyAction::AddRow => {
                self.commit_property_field(Some(window), cx);
                self.property_field = Some(Field::new(Target::New, Part::Key, ""));
                cx.notify();
            }
            PropertyAction::OpenTypeMenu { key } => {
                // * A second click on the same icon closes the menu.
                self.property_menu = if self.property_menu.as_deref() == Some(key.as_str()) {
                    None
                } else {
                    Some(key)
                };
                cx.notify();
            }
        }
    }

    /// Writes the open field into the document and closes it. Returns whether
    /// a field was open.
    /// The field the keys go to while one is open: a property row's, or a
    /// table cell's.
    ///
    /// Taking the field drops a selected pill: the selection is a step in
    /// the two-press Backspace gesture and survives nothing else, so typing
    /// or moving the caret puts focus back in the input. The gates that only
    /// ask whether a field is open go through [`Editor::field_kind`] instead,
    /// to leave the selection alone.
    pub(crate) fn active_field(&mut self) -> Option<&mut Field> {
        self.selected_pill = None;
        match self.property_field.as_mut() {
            Some(f) => Some(f),
            None => self.table_field.as_mut().map(|c| &mut c.field),
        }
    }

    /// A click on a table cell: commit whatever field was open, then take
    /// this cell's text into a new one.
    pub(crate) fn table_action(
        &mut self,
        action: TableAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let TableAction::Edit { span } = action;
        self.commit_property_field(Some(window), cx);
        // * The click arrives in the coordinates the widget was built from,
        //   which the commit may shift.
        let span = self
            .commit_table_field(Some(window), cx, Some(span.clone()))
            .unwrap_or(span);
        self.open_cell(span, cx);
    }

    /// Takes the cell's text into a field for typing.
    pub(crate) fn open_cell(&mut self, span: Range<usize>, cx: &mut Context<Self>) {
        let Some(was) = self.buf.text().get(span.clone()).map(str::to_string) else {
            return;
        };
        self.table_field = Some(CellEdit {
            span,
            field: Field::new(Target::New, Part::Value, was.clone()),
            was,
        });
        cx.notify();
    }

    /// Writes the open cell back into the document, if it changed.
    ///
    /// Obsidian re-pads a table only when a table command runs, so a cell
    /// edit replaces that cell's bytes and leaves the rest of the table
    /// exactly as it was. Returns `keep` shifted past the edit, so the cell
    /// a click asked for is still the right bytes afterwards.
    pub(crate) fn commit_table_field(
        &mut self,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
        keep: Option<Range<usize>>,
    ) -> Option<Range<usize>> {
        let CellEdit { span, was, field } = self.table_field.take()?;
        let text = table::cell_text(&field.text);
        // * The bytes have to be the ones the field was opened on: anything
        //   else means the document moved underneath, and the vault is the
        //   source of truth.
        if self.buf.text().get(span.clone()) != Some(was.as_str()) || text == was {
            cx.notify();
            return keep;
        }
        let grew = text.len() as isize - was.len() as isize;
        self.edit(span.clone(), &text, EditKind::Other, window, cx);
        cx.notify();
        keep.map(|k| {
            if k.start >= span.end {
                (k.start as isize + grew) as usize..(k.end as isize + grew) as usize
            } else {
                k
            }
        })
    }

    /// Drops the open cell without writing it.
    pub(crate) fn cancel_table_field(&mut self, cx: &mut Context<Self>) -> bool {
        let had = self.table_field.take().is_some();
        if had {
            cx.notify();
        }
        had
    }

    /// Opens the next cell in reading order (`dx`) or the cell below
    /// (`dy`), committing the one that is open. `false` when there is no
    /// such cell, so Tab out of the last one falls through to the editor.
    pub(crate) fn step_table_cell(
        &mut self,
        dx: isize,
        dy: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(span) = self.table_field.as_ref().map(|c| c.span.clone()) else {
            return false;
        };
        self.commit_table_field(Some(window), cx, None);
        // * The table is re-read after the commit: the cell's own bytes may
        //   have changed length.
        let Some(at) = table::table_at(self.buf.text(), span.start) else {
            return false;
        };
        let Some((row, col)) = at.cells.iter().enumerate().find_map(|(r, cells)| {
            cells
                .iter()
                .position(|s| s.start <= span.start && span.start <= s.end)
                .map(|c| (r, c))
        }) else {
            return false;
        };
        let next = if dy != 0 {
            row.checked_add_signed(dy)
                .and_then(|row| at.cells.get(row))
                .and_then(|cells| cells.get(col))
                .cloned()
        } else if dx > 0 {
            // * Past the last cell of a row, Tab goes on to the next row.
            match at.cells.get(row).map(|c| c.len()) {
                Some(len) if col + 1 < len => at.cells[row].get(col + 1).cloned(),
                _ => at.cells.get(row + 1).and_then(|c| c.first()).cloned(),
            }
        } else {
            match col.checked_sub(1) {
                Some(c) => at.cells[row].get(c).cloned(),
                None => row
                    .checked_sub(1)
                    .and_then(|r| at.cells.get(r))
                    .and_then(|c| c.last())
                    .cloned(),
            }
        };
        let Some(next) = next else {
            return false;
        };
        self.open_cell(next, cx);
        self.table_field.is_some()
    }

    /// Enter in a properties field. It commits, and in the trailing input of
    /// a list value Obsidian keeps that input focused so the next item can be
    /// typed straight away — "Enter in the trailing input adds an item", with
    /// the caret still there afterwards. The bytes are the commit's alone: a
    /// freshly opened input writes nothing until it is typed into.
    pub(crate) fn enter_property_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let again = match self.property_field.as_ref() {
            Some(field) => match (field.key(), field.list.as_ref()) {
                (Some(key), Some(list)) if list.index >= list.items.len() => Some(key.to_string()),
                _ => None,
            },
            None => None,
        };
        self.commit_property_field(Some(window), cx);
        // * The row is read after the commit: its list has just grown by the
        //   item that was typed, and the new input appends after it.
        if let Some(row) =
            again.and_then(|key| self.property_rows().into_iter().find(|row| row.key == key))
        {
            self.property_field = Some(row.append_field());
        }
        cx.notify();
    }

    /// Backspace in a properties field: Obsidian's two-press pill gesture in
    /// the trailing input of a list value, and a character delete anywhere
    /// else. The first press moves focus to the last pill and writes
    /// nothing, the second removes it through the same change the pill's own
    /// remove control makes, and repeating walks backwards a pill at a time;
    /// with the list empty the input has focus again and the press does
    /// nothing (see [`properties::PillStep`]).
    pub(crate) fn backspace_property_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self.selected_pill;
        let Some(field) = self.property_field.as_mut() else {
            return;
        };
        let key = field.key().map(str::to_string);
        let step = field.backspace(selected);
        self.selected_pill = match step {
            PillStep::Select(ix) => Some(ix),
            _ => None,
        };
        if let (PillStep::Remove(index), Some(key)) = (step, key) {
            let change = self
                .property_rows()
                .into_iter()
                .find(|row| row.key == key)
                .and_then(|row| properties::remove_item(&row, index));
            if let Some(change) = change {
                self.apply_property_changes(&[change], Some(window), cx);
            }
            // * The field carries the list as it was before the removal, so it
            //   is rebuilt from the row as it now stands: committing the stale
            //   snapshot would write the removed item back.
            self.property_field = self
                .property_rows()
                .into_iter()
                .find(|row| row.key == key)
                .map(|row| row.append_field());
        }
        cx.notify();
    }

    pub(crate) fn commit_property_field(
        &mut self,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) -> bool {
        self.selected_pill = None;
        let Some(field) = self.property_field.take() else {
            return false;
        };
        let ty = match field.key() {
            Some(key) => self.property_type_of(key),
            None => props::PropertyType::Text,
        };
        if let Some(change) = properties::commit(&field, ty) {
            self.apply_property_changes(&[change], window, cx);
        }
        cx.notify();
        true
    }

    /// Drops the open field without writing it.
    pub(crate) fn cancel_property_field(&mut self, cx: &mut Context<Self>) -> bool {
        self.selected_pill = None;
        let had = self.property_field.take().is_some();
        if had {
            cx.notify();
        }
        had
    }

    /// A click on a Live Preview widget: an embed and a table keep it
    /// (their links and cells take their own clicks), and a math block
    /// opens for editing at the clicked row. Returns true when the click
    /// was consumed.
    pub(crate) fn click_widget(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) -> bool {
        let Some(w) = self
            .layout
            .widgets
            .iter()
            .find(|w| w.bounds.contains(&position))
            .cloned()
        else {
            return false;
        };
        if w.embed {
            return true;
        }
        let lh = self.layout.line_height;
        let rows = w.lines.len().max(1);
        let row_h = (w.bounds.size.height / rows as f32).max(lh);
        let row = ((position.y - w.bounds.origin.y) / row_h).floor().max(0.) as usize;
        let line = (w.lines.start + row)
            .min(w.lines.end - 1)
            .min(self.line_count() - 1);
        self.buf.collapse_cursors();
        self.buf.head = self.buf.line_starts()[line];
        self.buf.anchor = self.buf.head;
        self.buf.goal = None;
        self.buf.marked = None;
        cx.notify();
        true
    }

    pub(crate) fn mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        if self.scrollbar_mouse_down(event.position, window, cx) {
            return;
        }
        if self.click_widget(event.position, cx) {
            return;
        }
        if self.click_rendered(event, window, cx) {
            return;
        }
        let Some(at) = self.offset_at_point(window, event.position) else {
            return;
        };
        self.buf.goal = None;
        self.buf.marked = None;
        if event.modifiers.alt && event.click_count == 1 {
            // * Alt+click adds a cursor (or removes the one it lands on).
            let mut cursors = self.buf.cursors();
            if let Some(ix) = cursors
                .sels
                .iter()
                .position(|s| s.is_empty() && s.head == at)
            {
                if cursors.sels.len() > 1 {
                    cursors.sels.remove(ix);
                    cursors.primary = cursors.primary.min(cursors.sels.len() - 1);
                }
            } else {
                cursors.sels.push(Sel::at(at));
                cursors.primary = cursors.sels.len() - 1;
            }
            self.buf.set_cursors(cursors);
            cx.notify();
            return;
        }
        if !event.modifiers.shift {
            self.buf.collapse_cursors();
        }
        match event.click_count {
            2 => {
                let word = self.buf.word_at(at);
                self.buf.anchor = word.start;
                self.buf.head = word.end;
            }
            n if n >= 3 => {
                let line = self.buf.cursor_at(at).line;
                self.buf.anchor = self.buf.line_starts()[line];
                self.buf.head = self
                    .buf
                    .line_starts()
                    .get(line + 1)
                    .copied()
                    .unwrap_or(self.buf.len());
            }
            _ => {
                self.buf.head = at;
                if !event.modifiers.shift {
                    self.buf.anchor = at;
                }
                self.dragging = true;
            }
        }
        cx.notify();
    }

    pub(crate) fn mouse_drag(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.thumb_drag.is_some() {
            self.scrollbar_drag(position, window, cx);
            return;
        }
        if !self.dragging {
            return;
        }
        let bounds = self.layout.bounds;
        let lh = self.layout.line_height;
        if position.y < bounds.origin.y {
            self.scroll_by(-lh, window, cx);
        } else if position.y > bounds.bottom_left().y {
            self.scroll_by(lh, window, cx);
        }
        let clamped = point(
            position.x.max(bounds.origin.x).min(bounds.bottom_right().x),
            position
                .y
                .max(bounds.origin.y)
                .min(bounds.bottom_right().y - px(1.)),
        );
        if let Some(at) = self.offset_at_point(window, clamped) {
            self.buf.head = at;
            cx.notify();
        }
    }
}
