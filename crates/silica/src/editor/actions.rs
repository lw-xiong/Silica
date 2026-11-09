//! Text editing actions: backspace/delete, tab/outdent, undo/redo,
//! clipboard cut/copy/paste (including file drops pasted as embeds) — the
//! methods of the split-out editing-actions section, unchanged apart from
//! living in their own module. Split from `editor.rs` per the audit's
//! module-size finding.

use std::sync::Arc;

use gpui::{ClipboardItem, Context, Window};
use silica_suggest::Accept;
use silica_text::{autopair, EditKind};

use crate::properties::{Field, Part, Target};
use crate::workspace::VaultState;
use silica_formats::FileKind;

use super::{
    attach, Backspace, Copy, Cut, Delete, DeleteWordLeft, DeleteWordRight, Editor, EditorEvent,
    Newline, NewlinePlain, Outdent, Paste, Redo, Tab, Undo,
};

impl Editor {
    pub(crate) fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        // * A properties field has the pill gesture behind it, so it takes the
        //   press through its own path.
        if self.property_field.is_some() {
            self.backspace_property_field(window, cx);
            return;
        }
        if let Some(f) = self.active_field() {
            let _ = f.backspace(None);
            cx.notify();
            return;
        }
        let kind = if self.selection().is_empty() {
            EditKind::Backspace
        } else {
            EditKind::Other
        };
        // * Backspace between an empty pair takes both characters.
        let opts = self.auto_pair();
        self.delete_all(
            move |buf, at| autopair::backspace_range(buf.text(), at, opts),
            kind,
            window,
            cx,
        );
    }

    pub(crate) fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(f) = self.active_field() {
            f.delete();
            cx.notify();
            return;
        }
        let kind = if self.selection().is_empty() {
            EditKind::Delete
        } else {
            EditKind::Other
        };
        self.delete_all(|buf, at| at..buf.next_char_boundary(at), kind, window, cx);
    }

    pub(crate) fn delete_word_left(
        &mut self,
        _: &DeleteWordLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_all(|buf, at| buf.word_left(at)..at, EditKind::Other, window, cx);
    }

    pub(crate) fn delete_word_right(
        &mut self,
        _: &DeleteWordRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_all(
            |buf, at| at..buf.word_right(at),
            EditKind::Other,
            window,
            cx,
        );
    }

    /// Enter: a newline that continues list and quote markers and the
    /// numbering of ordered lists, and clears or outdents an empty item.
    /// With several cursors, or a selection, a plain newline at each.
    pub(crate) fn newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        if self.accept_suggestion(Accept::Whole, window, cx) {
            return;
        }
        // * Enter in a properties field commits it, as Obsidian's does, and in
        //   a list's trailing input leaves that input focused for the next
        //   item.
        if self.property_field.is_some() {
            self.enter_property_field(window, cx);
            return;
        }
        // * In a table cell it commits and drops into the cell below, or just
        //   commits on the last row.
        if self.table_field.is_some() {
            if !self.step_table_cell(0, 1, window, cx) {
                self.commit_table_field(Some(window), cx, None);
            }
            return;
        }
        if self.buf.newline(self.tab_size, self.smart_indent_list) {
            self.after_edit(Some(window), cx, None);
        }
    }

    /// Shift+Enter: a plain newline.
    pub(crate) fn newline_plain(
        &mut self,
        _: &NewlinePlain,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.accept_suggestion(Accept::Typed, window, cx) {
            return;
        }
        self.insert_all("\n", EditKind::Other, window, cx);
    }

    /// Tab: indents list items and selected lines by one step (renumbering
    /// the ordered lists it touches), and inserts the step anywhere else.
    pub(crate) fn tab(&mut self, _: &Tab, window: &mut Window, cx: &mut Context<Self>) {
        if self.accept_suggestion(Accept::Inside, window, cx) {
            return;
        }
        // * Tab walks the cells of a table, committing each as it leaves it.
        if self.table_field.is_some() && self.step_table_cell(1, 0, window, cx) {
            return;
        }
        // * Tab commits a key field and moves to that property's value, as
        //   Obsidian's key input does (`enterkeyhint="next"`). From a value
        //   field it just commits, since there is nothing after it.
        if let Some(field) = self.property_field.clone() {
            self.commit_property_field(Some(window), cx);
            // * The key the commit left behind: what was typed, for both a
            //   rename and a newly named row.
            let key = field.text.trim().to_string();
            if field.part == Part::Key && !key.is_empty() {
                if let Some(row) = self.property_rows().into_iter().find(|r| r.key == key) {
                    self.property_field = Some(Field::new(
                        Target::Existing(key),
                        Part::Value,
                        row.edit_text,
                    ));
                }
            }
            cx.notify();
            return;
        }
        if self
            .buf
            .indent(self.use_tab, self.tab_size, self.smart_indent_list)
        {
            self.after_edit(Some(window), cx, None);
        }
    }

    /// Shift+Tab: removes one indentation step from the selected lines,
    /// renumbering the ordered lists it touches.
    pub(crate) fn outdent(&mut self, _: &Outdent, window: &mut Window, cx: &mut Context<Self>) {
        if self.table_field.is_some() && self.step_table_cell(-1, 0, window, cx) {
            return;
        }
        if self
            .buf
            .outdent(self.use_tab, self.tab_size, self.smart_indent_list)
        {
            self.after_edit(Some(window), cx, None);
        }
    }

    pub(crate) fn undo(&mut self, _: &Undo, window: &mut Window, cx: &mut Context<Self>) {
        if self.buf.undo() {
            self.after_edit(Some(window), cx, None);
        }
    }

    pub(crate) fn redo(&mut self, _: &Redo, window: &mut Window, cx: &mut Context<Self>) {
        if self.buf.redo() {
            self.after_edit(Some(window), cx, None);
        }
    }

    pub(crate) fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if self.field_kind().is_some() {
            return;
        }
        let ranges = self.buf.copy_ranges();
        if ranges.iter().any(|r| !r.is_empty()) {
            cx.write_to_clipboard(ClipboardItem::new_string(self.buf.clipboard_text(&ranges)));
        }
    }

    pub(crate) fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if self.field_kind().is_some() {
            return;
        }
        let ranges = self.buf.copy_ranges();
        if ranges.iter().any(|r| !r.is_empty()) {
            cx.write_to_clipboard(ClipboardItem::new_string(self.buf.clipboard_text(&ranges)));
            let edits = ranges.into_iter().map(|r| (r, String::new())).collect();
            self.edit_all(edits, EditKind::Other, Some(window), cx);
        }
    }

    /// Paste at every cursor; a clipboard with one line per cursor gives
    /// each its own line, the way CodeMirror pastes a multi-cursor copy.
    pub(crate) fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        // * Into the open field, which drops the newlines a row cannot hold.
        if let Some(field) = self.active_field() {
            if let Some(text) = item.text() {
                field.insert(&text);
                cx.notify();
            }
            return;
        }

        // * Attachment paste: image or external file paths
        if let Some(state) = self.suggest_vault.clone() {
            if !self.name.is_empty() {
                let image_entry = item.entries.iter().find_map(|entry| match entry {
                    gpui::ClipboardEntry::Image(img) => Some(img),
                    _ => None,
                });
                if let Some(img) = image_entry {
                    if self.paste_image_attachment(img, &state, window, cx) {
                        return;
                    }
                }

                let paths_entry = item.entries.iter().find_map(|entry| match entry {
                    gpui::ClipboardEntry::ExternalPaths(paths) => Some(paths),
                    _ => None,
                });
                if let Some(paths) = paths_entry {
                    if self.paste_files_attachment(&paths.0, &state, window, cx) {
                        return;
                    }
                }
            }
        }

        let Some(text) = item.text() else {
            return;
        };
        // * "Convert pasted HTML to Markdown": a page puts both forms on the
        //   clipboard, and the plain one has lost the markup. gpui never asks
        //   for the HTML, so it is read from the desktop's paste tool, and
        //   only when the clipboard really holds markup — a copy from a
        //   terminal leaves the text alone.
        let text = self
            .suggest_vault
            .as_ref()
            .filter(|state| state.app().auto_convert_html.unwrap_or(false))
            .and_then(|_| crate::clipboard_html::read())
            .and_then(|html| crate::html_to_markdown::convert(&html))
            .unwrap_or(text);
        if self.buf.paste(&text) {
            self.after_edit(Some(window), cx, None);
        }
    }

    pub(crate) fn paste_image_attachment(
        &mut self,
        img: &gpui::Image,
        state: &Arc<VaultState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if img.bytes.is_empty() {
            return false;
        }
        let folder = silica_vault::ops::attachment_folder(
            state.app().attachment_folder_path.as_deref(),
            &self.name,
        );
        let timestamp = chrono::Local::now().format("%Y%m%d%H%M%S");
        let stem = format!("Pasted image {timestamp}");
        let ext = match img.format {
            gpui::ImageFormat::Jpeg => "jpg",
            gpui::ImageFormat::Gif => "gif",
            gpui::ImageFormat::Webp => "webp",
            gpui::ImageFormat::Svg => "svg",
            gpui::ImageFormat::Bmp => "bmp",
            gpui::ImageFormat::Tiff => "tiff",
            _ => "png",
        };
        let target = state.vault().available_path(&folder, &stem, ext);
        match state.save_attachment(&target, &img.bytes) {
            Ok(changes) => {
                let resolver = state.vault().link_resolver();
                let written = silica_cache::links::link_path(
                    &resolver,
                    &target,
                    &self.name,
                    self.link_options.format,
                );
                let embed = if self.link_options.markdown_links {
                    format!("![]({})", written.replace(' ', "%20"))
                } else {
                    format!("![[{written}]]")
                };
                if self.buf.paste(&embed) {
                    self.after_edit(Some(window), cx, None);
                }
                cx.emit(EditorEvent::FilesChanged(changes));
                true
            }
            Err(e) => {
                cx.emit(EditorEvent::Reported(e.into()));
                false
            }
        }
    }

    pub(crate) fn paste_files_attachment(
        &mut self,
        paths: &[std::path::PathBuf],
        state: &Arc<VaultState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if paths.is_empty() {
            return false;
        }
        let folder = silica_vault::ops::attachment_folder(
            state.app().attachment_folder_path.as_deref(),
            &self.name,
        );
        let vault_root = state.vault().root().to_path_buf();
        let mut embeds = Vec::new();
        let mut all_changes = crate::workspace::Changes::default();

        for path in paths {
            if !path.is_file() {
                continue;
            }
            let target = if let Ok(rel) = path.strip_prefix(&vault_root) {
                rel.to_string_lossy().replace('\\', "/")
            } else {
                let filename = match path.file_name().and_then(|n| n.to_str()) {
                    Some(name) => name,
                    None => continue,
                };
                let (stem, ext) = match filename.rsplit_once('.') {
                    Some((s, e)) if !s.is_empty() => (s, e),
                    _ => (filename, ""),
                };
                let target = state.vault().available_path(&folder, stem, ext);
                let bytes = match std::fs::read(path) {
                    Ok(b) => b,
                    Err(e) => {
                        cx.emit(EditorEvent::Reported(
                            format!("Could not read file {}: {e}", path.display()).into(),
                        ));
                        continue;
                    }
                };
                match state.save_attachment(&target, &bytes) {
                    Ok(changes) => {
                        all_changes.extend(changes);
                        target
                    }
                    Err(e) => {
                        cx.emit(EditorEvent::Reported(e.into()));
                        continue;
                    }
                }
            };

            let resolver = state.vault().link_resolver();
            let written = silica_cache::links::link_path(
                &resolver,
                &target,
                &self.name,
                self.link_options.format,
            );
            let embed = if self.link_options.markdown_links {
                format!("![]({})", written.replace(' ', "%20"))
            } else {
                format!("![[{written}]]")
            };
            embeds.push(embed);
        }

        if embeds.is_empty() {
            return false;
        }

        let combined = embeds.join("\n");
        if self.buf.paste(&combined) {
            self.after_edit(Some(window), cx, None);
        }
        if !all_changes.files.is_empty() {
            cx.emit(EditorEvent::FilesChanged(all_changes));
        }
        true
    }

    pub(crate) fn handle_drop(
        &mut self,
        paths: &gpui::ExternalPaths,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.name.is_empty() {
            return;
        }
        let Some(state) = self.suggest_vault.clone() else {
            return;
        };
        self.paste_files_attachment(&paths.0, &state, window, cx);
    }

    /// "Insert image...": the desktop's open dialog, restricted to pictures,
    /// and each pick is stored in the vault and embedded at the cursor.
    pub(crate) fn attach_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.attach_from_disk(true, window, cx);
    }

    /// "Attach file...": the same dialog without the picture filter. An
    /// embed of a PDF or a zip shows the file in the note, and a click
    /// opens it.
    pub(crate) fn attach_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.attach_from_disk(false, window, cx);
    }

    /// Opens the dialog and hands what is picked to [`Editor::attach_paths`]
    /// inside the window again: a cancelled dialog changes nothing.
    fn attach_from_disk(&mut self, images_only: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.name.is_empty() {
            cx.emit(EditorEvent::Reported("Open a note first".into()));
            return;
        }
        if self.suggest_vault.is_none() {
            cx.emit(EditorEvent::Reported("Open a vault first".into()));
            return;
        }
        let picked = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(
                if images_only {
                    "Insert image"
                } else {
                    "Attach file"
                }
                .into(),
            ),
        });
        cx.spawn_in(window, async move |this, cx| {
            let paths = match picked.await {
                Ok(Ok(Some(paths))) => paths,
                Ok(Ok(None)) | Err(_) => return,
                Ok(Err(err)) => {
                    this.update(cx, |editor, cx| {
                        cx.emit(EditorEvent::Reported(
                            format!("Could not open the file dialog: {err}").into(),
                        ));
                        let _ = editor;
                    })
                    .ok();
                    return;
                }
            };
            this.update_in(cx, |editor, window, cx| {
                editor.attach_paths(&paths, images_only, window, cx)
            })
            .ok();
        })
        .detach();
    }

    /// Stores each of `paths` in the vault, under its own name made free,
    /// and embeds them at the cursor. A file that cannot be read, stored
    /// or named is reported and skipped; the rest still land.
    pub(crate) fn attach_paths(
        &mut self,
        paths: &[std::path::PathBuf],
        images_only: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if paths.is_empty() || self.name.is_empty() {
            return;
        }
        let Some(state) = self.suggest_vault.clone() else {
            return;
        };
        let folder = silica_vault::ops::attachment_folder(
            state.app().attachment_folder_path.as_deref(),
            &self.name,
        );
        let mut embeds = Vec::new();
        let mut all_changes = crate::workspace::Changes::default();
        for path in paths {
            if !path.is_file() {
                continue;
            }
            let name = match attach::stored_name(path) {
                Ok(name) => name,
                Err(refused) => {
                    cx.emit(EditorEvent::Reported(refused.message().into()));
                    continue;
                }
            };
            if images_only && !attach::is_image_name(&name) {
                cx.emit(EditorEvent::Reported(
                    format!("{name} is not an image").into(),
                ));
                continue;
            }
            let target = match attach::first_free(&folder, &name, |rel| state.vault().exists(rel)) {
                Ok(target) => target,
                Err(refused) => {
                    cx.emit(EditorEvent::Reported(refused.message().into()));
                    continue;
                }
            };
            let bytes = match std::fs::read(path) {
                Ok(bytes) => bytes,
                Err(e) => {
                    cx.emit(EditorEvent::Reported(
                        format!("Could not read {}: {e}", path.display()).into(),
                    ));
                    continue;
                }
            };
            match state.save_attachment(&target, &bytes) {
                Ok(changes) => all_changes.extend(changes),
                Err(e) => {
                    cx.emit(EditorEvent::Reported(e.into()));
                    continue;
                }
            }
            let resolver = state.vault().link_resolver();
            let written = silica_cache::links::link_path(
                &resolver,
                &target,
                &self.name,
                self.link_options.format,
            );
            embeds.push(attach::embed(&self.link_options, &written));
        }
        if embeds.is_empty() {
            return;
        }
        if self.buf.paste(&embeds.join("\n")) {
            self.after_edit(Some(window), cx, None);
        }
        if !all_changes.files.is_empty() {
            cx.emit(EditorEvent::FilesChanged(all_changes));
        }
    }

    /// The embed the cursor sits on, if any. The toolbar's "Remove" row is
    /// only offered when this finds one, so the same question decides what
    /// the button does and whether it can do it.
    pub(crate) fn attachment_under_cursor(&self) -> Option<attach::Embed> {
        if self.name.is_empty() {
            return None;
        }
        attach::attachment_at(self.buf.text(), self.buf.selection().start)
    }

    /// "Remove attachment": the file behind the embed under the cursor goes
    /// to the trash and the embed leaves the note with it. A toolbar that
    /// is asked to remove one removes one.
    pub(crate) fn remove_attachment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.suggest_vault.clone() else {
            return;
        };
        let Some(embed) = self.attachment_under_cursor() else {
            cx.emit(EditorEvent::Reported(
                "Put the cursor on an attachment to remove it".into(),
            ));
            return;
        };
        let Some(target) = attach::embed_target(&embed) else {
            cx.emit(EditorEvent::Reported(
                "That embed points at this note, not at a file".into(),
            ));
            return;
        };
        let resolver = state.vault().link_resolver();
        let Some(path) = resolver.resolve(&target, &self.name).map(str::to_string) else {
            cx.emit(EditorEvent::Reported(
                format!("{target} is not a file in this vault").into(),
            ));
            return;
        };
        if state.vault().absolute(&path).is_dir() {
            cx.emit(EditorEvent::Reported(
                format!("{path} is a folder, not an attachment").into(),
            ));
            return;
        }
        // * A basename match can resolve to a note (embeds of `[[Note]]`
        //   resolve across the vault); trashing a note from the attachment
        //   toolbar would be an unconfirmed data loss, so refuse notes here.
        if FileKind::from_path(&path).is_markdown() {
            cx.emit(EditorEvent::Reported(
                "That embed points at a note, not an attachment".into(),
            ));
            return;
        }
        let changes = match state.trash(&path) {
            Ok(changes) => changes,
            Err(e) => {
                cx.emit(EditorEvent::Reported(e.into()));
                return;
            }
        };
        let range = attach::removal_range(self.buf.text(), embed.span);
        self.edit_all(
            vec![(range, String::new())],
            EditKind::Other,
            Some(window),
            cx,
        );
        cx.emit(EditorEvent::FilesChanged(changes));
    }
}
