//! File operations behind the workspace: creating, renaming, moving,
//! duplicating and trashing notes and folders, link rewriting after a
//! rename, and the attachment target folder — the methods of the split-out
//! file-operations section, unchanged apart from living in their own
//! module. Split from `workspace.rs` per the audit's module-size finding.

use super::*;

impl Workspace {

    pub(crate) fn open_prompt(
        &mut self,
        op: PendingOp,
        spec: PromptSpec,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending = Some(op);
        self.close_editor_suggest(cx);
        self.modal = Modal::Prompt;
        self.prompt.update(cx, |p, cx| p.open(spec, window, cx));
        cx.notify();
    }

    /// Shows a message that goes away on its own, the way Obsidian's
    /// Notice does. Everything a command has to say comes through here;
    /// the status bar keeps only what stays true.
    /// The explorer menu's "Copy path": the **vault-relative** path, which is
    /// the only path the rest of the app speaks and the one the vault is the
    /// source of truth for. Silent, because Obsidian shows no notice for it
    /// and a notice Obsidian does not show is a divergence of its own. The
    /// capture records the item but not its payload, so the shape of the
    /// string is marked OBSERVE in
    /// [[Specification#The file explorer's context menus]].
    pub(crate) fn copy_path(&mut self, path: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(path.to_string()));
    }

    /// Obsidian's `workspace:copy-path` and `workspace:copy-full-path`, which
    /// only exist as menu items over there but are commands in every sense:
    /// the path of the open note, vault-relative or the host path, and a
    /// spoken refusal with no note open rather than a clipboard holding the
    /// last thing it held.
    pub(crate) fn copy_current_path(&mut self, full: bool, cx: &mut Context<Self>) {
        let Some(current) = self.current.clone() else {
            self.report("No note is open", cx);
            return;
        };
        let text = if full {
            match self.vault_path(&current) {
                Ok(path) => path.display().to_string(),
                Err(_) => current,
            }
        } else {
            current
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    /// The explorer menu's "Open in default app": the file handed to the
    /// desktop, one command per platform and no crate for it, so the
    /// dependency tree is unchanged. The child is spawned and let go -- the
    /// handler it starts outlives this process on every platform -- and only a
    /// failure to spawn is reported, since nothing after that is Silica's to
    /// know.
    pub(crate) fn open_in_default_app(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let absolute = state.vault().absolute(path);
        // * `cmd /C start` was here on Windows, which made the file name shell
        //   syntax: a note called `a&calc.exe.md` ran a command on being opened.
        //   The system's own open call takes the path as data instead.
        let outcome = silica_plugins::launch::open_path(&absolute);
        if let Err(err) = outcome {
            self.report(format!("Could not open {path}: {err}"), cx);
        }
    }

    /// "Show in system explorer": select the file in the desktop's file
    /// manager via D-Bus `FileManager1.ShowItems`, with opening the
    /// containing folder as fallback.
    pub(crate) fn show_in_system_explorer(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let absolute = state.vault().absolute(path);
        // * D-Bus first: `org.freedesktop.FileManager1.ShowItems` selects
        //   the file(s) in the file manager. It has to *finish*, not just
        //   spawn: a session bus with nothing answering the name exits
        //   dbus-send non-zero a moment later, and `spawn()` alone would
        //   report success and never run the fallback.
        let selected = show_items(&absolute);
        if selected {
            return;
        }
        // * Fallback: open the containing folder in the file manager.
        let folder = absolute.parent().map(std::path::Path::to_path_buf);
        match folder {
            Some(folder) => {
                if let Err(err) = silica_plugins::launch::open_path(&folder) {
                    self.report(
                        format!("Could not show {path} in system explorer: {err}"),
                        cx,
                    );
                }
            }
            None => self.report(
                format!("Could not show {path} in system explorer: no parent folder"),
                cx,
            ),
        }
    }

    /// The explorer menu's "Search in folder": the Search pane scoped with the
    /// `path:` operator the query grammar already has. The trailing slash
    /// keeps `Notes/` from also matching `Notes archive/`. What Obsidian puts
    /// in its own box here was not captured, so the query is marked OBSERVE in
    /// [[Specification#The file explorer's context menus]].
    pub(crate) fn search_in_folder(&mut self, folder: &str, window: &mut Window, cx: &mut Context<Self>) {
        let query = match folder.is_empty() {
            true => String::new(),
            false => format!("path:\"{folder}/\""),
        };
        self.show_search(&query, Some(window), cx);
    }

    pub(crate) fn report(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        let id = self.notices.push(message.into());
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(NOTICE_LIFETIME).await;
            this.update(cx, |ws, cx| {
                ws.notices.expire(id);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Creates `Untitled.md` (or the next free number) in `folder`, opens it
    /// for editing and asks for its name, the way Ctrl+N does in Obsidian.
    pub(crate) fn new_note(&mut self, folder: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let vault = state.vault();
        let name = if vault
            .read_config::<ZkPrefixerConfig>("zk-prefixer.json")
            .unwrap_or_default()
            .unwrap_or_default()
            .enabled
            .unwrap_or(false)
        {
            let config = vault
                .read_config::<ZkPrefixerConfig>("zk-prefixer.json")
                .unwrap_or_default()
                .unwrap_or_default();
            let format = config
                .format
                .as_deref()
                .unwrap_or(ZK_PREFIXER_DEFAULT_FORMAT);
            let local = chrono::Local::now();
            let naive = local.naive_local();
            let stamp = templates::Stamp {
                year: chrono::Datelike::year(&naive) as i64,
                month: chrono::Datelike::month(&naive),
                day: chrono::Datelike::day(&naive),
                hour: chrono::Timelike::hour(&naive),
                minute: chrono::Timelike::minute(&naive),
                second: chrono::Timelike::second(&naive),
            };
            format!("{} Untitled", templates::moment::format(format, &stamp))
        } else {
            "Untitled".to_string()
        };
        let path = vault.available_path(folder, &name, "md");
        match state.create_note(&path, "") {
            Ok(changes) => {
                self.after_changes(changes, cx);
                self.open_file(path.clone(), None, cx);
                let mode = self.editing_mode();
                self.set_mode(mode, window, cx);
                self.start_rename(&path, window, cx);
            }
            Err(err) => self.report(err, cx),
        }
    }

    /// `canvas:new-file`: `Untitled.canvas` (or the next free number) in
    /// `folder` (the vault root for the command, the clicked folder for the
    /// explorer's menu) as an empty file, which is what Obsidian writes until
    /// something is put in the canvas (see [[Specification#Observed canvas
    /// write behavior]]), opened the way a new note is: in a new tab unless
    /// the active one is empty.
    pub(crate) fn new_canvas(&mut self, folder: &str, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let path = state.vault().available_path(folder, "Untitled", "canvas");
        match state.create_note(&path, "") {
            Ok(changes) => {
                self.after_changes(changes, cx);
                if self.current.is_some() {
                    self.new_tab(cx);
                }
                self.open_file(path.clone(), None, cx);
                self.report(format!("Created {path}"), cx);
            }
            Err(err) => self.report(err, cx),
        }
    }

    /// `bases:new-file`: `Untitled.base` (or the next free number) in
    /// `folder` as a **zero-byte** file, opened the way a new canvas is.
    ///
    /// The empty file is Obsidian's own behaviour, not a shortcut: the probe
    /// of 2026-10-25 watched `bases:new-file` write nothing at all, because
    /// the parsed query of an empty file is already one default view, and
    /// only when that view is closed does Obsidian write `views:` / `- type:
    /// table` / `name: View` (see [[Specification#Observed Bases write
    /// behavior, Obsidian 1.14.5 (installer build 1.13.7)]]). So a base
    /// created here and never opened in Obsidian is byte-identical to one
    /// Obsidian created and never opened.
    ///
    /// Obsidian then opens the new base in its Bases view, and so does this:
    /// `show_file`'s [`FileKind::Base`] arm hands the empty file to
    /// [`BasesView`], whose parse of no bytes at all is one default view.
    pub(crate) fn new_base(&mut self, folder: &str, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let path = state.vault().available_path(folder, "Untitled", "base");
        match state.create_note(&path, "") {
            Ok(changes) => {
                self.after_changes(changes, cx);
                if self.current.is_some() {
                    self.new_tab(cx);
                }
                self.open_file(path.clone(), None, cx);
                self.report(format!("Created {path}"), cx);
            }
            Err(err) => self.report(err, cx),
        }
    }

    /// `bases:add-view`: appends a view to the showing base and shows it.
    ///
    /// Observed: the appended view is named `View 2` and Obsidian opens its
    /// Configure view page. The name and the write are
    /// [`BasesView::add_view`]'s; the Configure view page is the toolbar's
    /// views panel, which this cannot open yet, so the new view is only
    /// shown. Nothing is announced — Obsidian raises no Notice for it, and
    /// the autosave the edit schedules says the file was written.
    ///
    /// With no base showing it does nothing at all, the way Obsidian's own
    /// check callback keeps the command off the palette.
    pub(crate) fn add_base_view(&mut self, cx: &mut Context<Self>) {
        if self.main_view(cx) != MainView::Bases {
            return;
        }
        self.bases.update(cx, |b, cx| b.add_view(cx));
    }

    /// `bases:change-view`: a chooser over the showing base's own views,
    /// which shows the one picked.
    ///
    /// **Unprobed**: the command was never driven, so only its palette name
    /// — "Bases: Switch view..." — is observed. The ellipsis is Obsidian's
    /// mark for a command that opens a suggester, so this is the chooser
    /// `canvas:jump-to-group` opens, over the view names, and the
    /// placeholder and the empty line are this app's rather than observed.
    pub(crate) fn change_base_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.main_view(cx) != MainView::Bases {
            return;
        }
        let rows: Vec<ChooserRow> = self
            .bases
            .read(cx)
            .view_names()
            .into_iter()
            .enumerate()
            .map(|(ix, label)| ChooserRow {
                label,
                value: ix.to_string(),
            })
            .collect();
        // * A base that did not parse has no views to switch between, and a
        //   chooser of nothing is not what the command does.
        if rows.is_empty() {
            return;
        }
        self.chooser_purpose = ChooserPurpose::BasesView;
        self.modal = Modal::Chooser;
        self.chooser.update(cx, |c, cx| {
            c.open(
                rows,
                "Type name of a view...",
                "No views in this base",
                window,
                cx,
            )
        });
        cx.notify();
    }

    /// `bases:copy-table`: the showing view's visible columns on the
    /// clipboard as TSV.
    ///
    /// Observed: the bytes are one header row of display names, tabs between
    /// the cells and no trailing newline
    /// (`status\tPrice per unit\ntodo\t0.02\ntodo\t0.06`), and the notice is
    /// "Copied to your clipboard". The command **needs the view focused**:
    /// run from the palette with the keyboard elsewhere it reports success
    /// and shows "Unable to copy to your clipboard" instead, with nothing
    /// copied. Both outcomes are here, on the rule that the keyboard has to
    /// be in the base; which branch the palette itself lands in is this
    /// app's business rather than Obsidian's, since closing a modal here
    /// hands the keyboard back to the main area.
    ///
    /// With no base showing it does nothing at all, and says nothing.
    pub(crate) fn copy_base_table(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.main_view(cx) != MainView::Bases {
            return;
        }
        let Some(text) = self.bases.read(cx).table_tsv() else {
            return;
        };
        if self.modal != Modal::None || !self.bases.focus_handle(cx).is_focused(window) {
            self.report("Unable to copy to your clipboard", cx);
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.report("Copied to your clipboard", cx);
    }

    /// `bases:insert`: another zero-byte base, and `![[Untitled 1.base]]` at
    /// the cursor.
    ///
    /// Observed: the file is a second `Untitled.base` — `Untitled 1.base` in
    /// the probe, because `Untitled.base` was already there — created the
    /// same way `bases:new-file` creates one, and the embed lands **on a
    /// line of its own** at the cursor. So the line is broken where the
    /// cursor sits: a newline goes before the embed when anything precedes
    /// it on that line, and after it when anything follows.
    ///
    /// This is the one `bases:*` command that runs in a note rather than in
    /// a base, which is why it is guarded like every other editor command.
    pub(crate) fn insert_base(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editing_or_report("bases:insert", window, cx) {
            return;
        }
        let Some(state) = self.state.clone() else {
            return;
        };
        let path = state.vault().available_path("", "Untitled", "base");
        let changes = match state.create_note(&path, "") {
            Ok(changes) => changes,
            Err(err) => return self.report(err, cx),
        };
        self.after_changes(changes, cx);
        // * The link is the file's own name with its extension, the way a
        //   non-markdown file is always linked; the new base is in the vault
        //   root, as `bases:new-file`'s is.
        let name = path.rsplit('/').next().unwrap_or(&path).to_string();
        let (before, after) = {
            let editor = self.editor.read(cx);
            let text = editor.text();
            let at = editor.selection();
            let head = &text[..at.start.min(text.len())];
            let tail = &text[at.end.min(text.len())..];
            (
                !head.rsplit('\n').next().unwrap_or(head).is_empty(),
                !tail.split('\n').next().unwrap_or(tail).is_empty(),
            )
        };
        let insert = format!(
            "{}![[{name}]]{}",
            if before { "\n" } else { "" },
            if after { "\n" } else { "" }
        );
        self.editor
            .update(cx, |e, cx| e.insert_at_cursors(&insert, window, cx));
        cx.notify();
    }

    /// `bases:add-item`: a new note for the showing base to pick up.
    ///
    /// **Unprobed**: the command was never driven. What the base carries for
    /// it is `newItemFolder` and `newItemTemplate` — two keys the published
    /// schema never mentions — so the reading is a new `Untitled.md` in that
    /// folder from that template, opened; the base keeps the tab it is in
    /// and the note takes a new one. The template is read the way a daily
    /// note's is, placeholders and all, and nothing is written into the
    /// `.base` itself.
    ///
    /// A base that names no folder gets the one it is in, which is where a
    /// note it queries is most likely to belong.
    pub(crate) fn add_base_item(&mut self, cx: &mut Context<Self>) {
        if self.main_view(cx) != MainView::Bases {
            return;
        }
        let Some(state) = self.state.clone() else {
            return;
        };
        let Some(item) = self.bases.read(cx).new_item() else {
            return;
        };
        let folder = item.folder.unwrap_or_else(|| {
            self.bases
                .read(cx)
                .path()
                .and_then(|p| p.rsplit_once('/'))
                .map(|(dir, _)| dir.to_string())
                .unwrap_or_default()
        });
        let path = state.vault().available_path(&folder, "Untitled", "md");
        let text = match item.template.as_deref() {
            Some(template) => self.daily_template_text(&state, template, &path, cx),
            None => String::new(),
        };
        match state.create_note(&path, &text) {
            Ok(changes) => {
                self.after_changes(changes, cx);
                self.open_in_new_tab(path.clone(), None, cx);
                self.report(format!("Created {path}"), cx);
            }
            Err(err) => self.report(err, cx),
        }
    }

    /// A cell of a base was typed into: sets `key` to `value` in the note
    /// that row stands for.
    ///
    /// The base view asks for this rather than writing the note itself,
    /// because a base's row is **another file**: it may be open in a buffer
    /// with unsaved edits, and a write behind that buffer's back would be
    /// thrown away by the buffer's own next save. So the guard is the one the
    /// unlinked-mention write uses — a buffer holding this note must have
    /// exactly the bytes on disk, or nothing is written and the user is told
    /// why — and the change is planned over the note's own text, which keeps
    /// its line endings, its BOM and every key the edit did not name.
    pub(crate) fn set_note_property(
        &mut self,
        path: String,
        key: String,
        value: String,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let absolute = match state.vault().write_path(&path) {
            Ok(absolute) => absolute,
            Err(err) => {
                self.report(format!("{path} not written: {err}"), cx);
                return;
            }
        };
        let on_disk = match std::fs::read_to_string(&absolute) {
            Ok(text) => text,
            Err(err) => {
                self.report(format!("Could not read {path}: {err}"), cx);
                return;
            }
        };
        // * Any group's editor may be holding the note, not just the active
        //   one, and a buffer that differs from the disk is unsaved work.
        if self.buffer_differs(&path, &on_disk, cx) {
            self.report(format!("{path} has unsaved edits; {key} not set"), cx);
            return;
        }
        let written = {
            let types = state.property_types();
            crate::bases::edit::write_property(&on_disk, &key, &value, &types)
        };
        // * An unchanged value writes nothing at all, which is what keeps a
        //   committed field from touching a file it did not change.
        let Some(written) = written else {
            return;
        };
        if let Err(err) = silica_vault::write_no_follow(&absolute, written.as_bytes()) {
            self.report(format!("Could not write {path}: {err}"), cx);
            return;
        }
        self.report(format!("{key} set in {path}"), cx);
        // * The index is what every base resolves against, so the table comes
        //   back through the same path an outside edit would take.
        self.files_changed(vec![absolute], cx);
    }

    /// Whether any group's editor holds `path` with bytes that are not
    /// `on_disk` — that is, with unsaved edits a write would discard.
    pub(crate) fn buffer_differs(&self, path: &str, on_disk: &str, cx: &mut Context<Self>) -> bool {
        let mine = {
            let editor = self.editor.read(cx);
            editor.name.as_ref() == path && editor.text() != on_disk
        };
        mine || self.groups.iter().any(|group| {
            let editor = group.editor.read(cx);
            editor.name.as_ref() == path && editor.text() != on_disk
        })
    }

    /// Writes the base view's file now if the view changed it, through the
    /// same checks as the canvas's save: the path is checked for links, and a
    /// file that is not the bytes the view read or last wrote is left alone,
    /// since someone else wrote it.
    ///
    /// The bytes are `Base::to_yaml` — Obsidian's own canonical form, which
    /// is what it writes on a toolbar change and on close — and a base that
    /// failed to parse answers no contents at all, so a broken file is never
    /// written over.
    pub(crate) fn flush_bases_save(&mut self, state: &Arc<VaultState>, cx: &mut Context<Self>) {
        let (dirty, path) = {
            let bases = self.bases.read(cx);
            (bases.is_dirty(), bases.path().map(str::to_string))
        };
        let Some(path) = path.filter(|_| dirty) else {
            return;
        };
        let absolute = match state.vault().write_path(&path) {
            Ok(absolute) => absolute,
            Err(err) => {
                self.report(format!("{path} not saved: {err}"), cx);
                return;
            }
        };
        match std::fs::read_to_string(&absolute) {
            Ok(on_disk) if self.bases.read(cx).matches_file(&on_disk) => {}
            Ok(_) => {
                self.report(
                    format!("{path} changed on disk; not overwritten, your edits are still here"),
                    cx,
                );
                return;
            }
            Err(err) => {
                self.report(
                    format!("Could not read {path}: {err}; not overwritten, your edits are kept"),
                    cx,
                );
                return;
            }
        }
        let Some(contents) = self.bases.read(cx).file_contents() else {
            return;
        };
        match silica_vault::write_no_follow(&absolute, contents.as_bytes()) {
            Ok(()) => {
                self.bases.update(cx, |b, _| b.mark_saved(contents));
                self.report(format!("{path} saved"), cx);
            }
            Err(err) => self.report(format!("Could not save {path}: {err}"), cx),
        }
        cx.notify();
    }

    /// Writes the canvas view's file now if it has unsaved edits, through
    /// the same checks as the editor's save: the path is checked for links,
    /// and a file that is not the bytes the view read or last wrote is left
    /// alone, since someone else wrote it.
    pub(crate) fn flush_canvas_save(&mut self, state: &Arc<VaultState>, cx: &mut Context<Self>) {
        let (dirty, path) = {
            let canvas = self.canvas.read(cx);
            (canvas.is_dirty(), canvas.path().map(str::to_string))
        };
        let Some(path) = path.filter(|_| dirty) else {
            return;
        };
        let absolute = match state.vault().write_path(&path) {
            Ok(absolute) => absolute,
            Err(err) => {
                self.report(format!("{path} not saved: {err}"), cx);
                return;
            }
        };
        match std::fs::read_to_string(&absolute) {
            Ok(on_disk) if self.canvas.read(cx).matches_file(&on_disk) => {}
            Ok(_) => {
                self.report(
                    format!("{path} changed on disk; not overwritten, your edits are still here"),
                    cx,
                );
                return;
            }
            Err(err) => {
                self.report(
                    format!("Could not read {path}: {err}; not overwritten, your edits are kept"),
                    cx,
                );
                return;
            }
        }
        let contents = self.canvas.read(cx).to_json();
        match silica_vault::write_no_follow(&absolute, contents.as_bytes()) {
            Ok(()) => {
                self.canvas.update(cx, |c, _| c.mark_saved(contents));
                self.report(format!("{path} saved"), cx);
            }
            Err(err) => self.report(format!("Could not save {path}: {err}"), cx),
        }
        cx.notify();
    }

    /// A canvas view moved: the tab showing it records where it looks, so
    /// `workspace.json` gets it whether or not the tab is live when the
    /// layout is written. The view may belong to a group that is not the
    /// active one — the wheel reaches a pane without activating it.
    pub(crate) fn record_canvas_view_state(
        &mut self,
        canvas: &Entity<CanvasView>,
        view_state: Value,
        cx: &mut Context<Self>,
    ) {
        let path = canvas.read(cx).path().map(str::to_string);
        if path.is_none() {
            return;
        }
        if *canvas == self.canvas {
            if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                if self.current == path {
                    tab.view_state = Some(view_state);
                }
            }
            return;
        }
        if let Some(group) = self.groups.iter_mut().find(|g| g.canvas == *canvas) {
            if let Some(tab) = group.tabs.get_mut(group.active_tab) {
                if tab.path == path {
                    tab.view_state = Some(view_state);
                }
            }
        }
    }

    /// Creates the note a link or the switcher names and opens it.
    pub(crate) fn create_note_for(&mut self, name: &str, window: Option<&mut Window>, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let path = state.new_note_path(name, self.current.as_deref());
        match state.create_note(&path, "") {
            Ok(changes) => {
                self.after_changes(changes, cx);
                self.open_file(path.clone(), None, cx);
                if let Some(window) = window {
                    let mode = self.editing_mode();
                    self.set_mode(mode, window, cx);
                }
                self.report(format!("Created {path}"), cx);
            }
            Err(err) => self.report(err, cx),
        }
    }

    pub(crate) fn create_from_link(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.create_note_for(name, Some(window), cx);
    }

    pub(crate) fn start_rename(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let is_folder = self
            .state
            .as_ref()
            .map(|s| s.vault().absolute(path).is_dir())
            .unwrap_or(false);
        let name = path.rsplit('/').next().unwrap_or(path);
        let initial = if is_folder { name } else { basename(name) };
        self.open_prompt(
            PendingOp::Rename(path.to_string()),
            PromptSpec {
                title: if is_folder {
                    "Rename folder".into()
                } else {
                    "Rename file".into()
                },
                detail: Some(path.to_string().into()),
                input: Some(initial.to_string()),
                placeholder: "Name".into(),
                confirm: "Rename".into(),
                ..PromptSpec::default()
            },
            window,
            cx,
        );
    }

    /// `silica:export-site`: asks where the site goes, with the vault's
    /// sibling folder as the pre-filled answer.
    pub(crate) fn start_export_site(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        watch: bool,
    ) {
        // * The default folder is read through the state's borrow, which
        //   ends before `open_prompt` takes `self` back.
        let default = match self.state.as_ref() {
            Some(state) => {
                let vault = state.vault();
                let root = vault.root();
                let name = root
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "vault".into());
                match root.parent() {
                    Some(parent) if !parent.as_os_str().is_empty() => parent
                        .join(format!("{name}-site"))
                        .to_string_lossy()
                        .into_owned(),
                    _ => format!("{name}-site"),
                }
            }
            None => return,
        };
        self.open_prompt(
            if watch {
                PendingOp::ExportSiteWatch
            } else {
                PendingOp::ExportSite
            },
            PromptSpec {
                title: if watch {
                    "Export vault as site (watching)".into()
                } else {
                    "Export vault as site".into()
                },
                detail: Some(
                    if watch {
                        "Every note renders to HTML; links and attachments come along. The folder refreshes as the vault changes — run this again to stop."
                    } else {
                        "Every note renders to HTML; links and attachments come along."
                    }
                    .into(),
                ),
                input: Some(default),
                placeholder: "Folder".into(),
                confirm: "Export".into(),
                ..PromptSpec::default()
            },
            window,
            cx,
        );
    }

    /// `silica:toggle-graph-pane`: the graph in the right dock, beside the
    /// note — Obsidian's graph side pane. Asked for with the pane already
    /// showing it, the dock closes instead. The dock comes back first: a
    /// pane that is not painted is outside the dispatch tree.
    pub(crate) fn toggle_graph_pane(&mut self, cx: &mut Context<Self>) {
        if self.right_tab == RightTab::Graph && self.right_visible {
            self.right_visible = false;
            cx.notify();
        } else {
            self.right_visible = true;
            self.select_right_tab(RightTab::Graph, cx);
        }
    }

    /// The watching export: a first run starts a background pass over the
    /// vault, a second stops it. The thread owns its watcher, so the flag
    /// is all a stop needs; taking the `Arc` out of `site_watch` is what
    /// makes the command a toggle.
    pub(crate) fn toggle_site_watch(&mut self, root: PathBuf, out: PathBuf, cx: &mut Context<Self>) {
        if let Some(stop) = self.site_watch.take() {
            stop.store(true, Ordering::Relaxed);
            self.report("Stopped watching the site export.", cx);
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        self.site_watch = Some(stop.clone());
        std::thread::spawn(move || crate::site::watch_loop(root, out, stop));
        self.report("Watching the vault — the site re-exports on change.", cx);
    }

    /// `silica:open-exported-site`: the last export's index.html in the
    /// default browser — the "preview" half of the site export, with no
    /// re-export and no temp server to leave running.
    pub(crate) fn open_exported_site(&mut self, cx: &mut Context<Self>) {
        let Some(out) = self.last_site_export.clone() else {
            self.report(
                "No export to open yet — run Export vault as site first.",
                cx,
            );
            return;
        };
        let index = out.join("index.html");
        if !index.exists() {
            self.report(
                format!("No index.html in {} — export it again.", out.display()),
                cx,
            );
            return;
        }
        cx.open_url(&crate::site::index_url(&out));
        self.report(format!("Opening {} in the browser.", index.display()), cx);
    }

    pub(crate) fn start_move(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let is_folder = state.vault().absolute(path).is_dir();
        let own_prefix = format!("{path}/");
        let parent = path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        let mut suggestions = vec!["/".to_string()];
        suggestions.extend(
            state
                .vault()
                .folders()
                .iter()
                .filter(|f| *f != path && !f.starts_with(&own_prefix) && f.as_str() != parent)
                .cloned(),
        );
        if parent.is_empty() {
            suggestions.remove(0);
        }
        self.open_prompt(
            PendingOp::Move(path.to_string()),
            PromptSpec {
                title: if is_folder {
                    "Move folder to...".into()
                } else {
                    "Move file to...".into()
                },
                detail: Some(path.to_string().into()),
                input: Some(String::new()),
                placeholder: "Type a folder".into(),
                suggestions,
                confirm: "Move".into(),
                ..PromptSpec::default()
            },
            window,
            cx,
        );
    }

    pub(crate) fn start_new_folder(&mut self, parent: &str, window: &mut Window, cx: &mut Context<Self>) {
        let initial = self
            .state
            .as_ref()
            .map(|s| s.vault().available_path(parent, "Untitled", ""))
            .unwrap_or_else(|| "Untitled".to_string());
        let initial = initial.rsplit('/').next().unwrap_or(&initial).to_string();
        self.open_prompt(
            PendingOp::NewFolder {
                parent: parent.to_string(),
            },
            PromptSpec {
                title: "New folder".into(),
                detail: Some(if parent.is_empty() {
                    "In the vault folder".into()
                } else {
                    format!("In {parent}").into()
                }),
                input: Some(initial),
                placeholder: "Folder name".into(),
                confirm: "Create".into(),
                ..PromptSpec::default()
            },
            window,
            cx,
        );
    }

    pub(crate) fn start_delete(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        if !state.confirm_delete() {
            self.delete_now(path, window, cx);
            return;
        }
        let where_to = match state.trash_option() {
            TrashOption::System => "It will be moved to the system trash.",
            TrashOption::Local => "It will be moved to the vault's .trash folder.",
            TrashOption::None => "It will be deleted permanently.",
        };
        let name = path.rsplit('/').next().unwrap_or(path);
        self.open_prompt(
            PendingOp::Delete(path.to_string()),
            PromptSpec {
                title: format!("Delete \"{name}\"?").into(),
                detail: Some(where_to.into()),
                input: None,
                confirm: "Delete".into(),
                ..PromptSpec::default()
            },
            window,
            cx,
        );
    }

    /// The explorer menu's "Merge entire file with...": the open note takes
    /// another note in. The chooser lists the vault's other notes; the merge
    /// itself is [`merge::merge`] against the two files on disk, and the
    /// result lands in the open note's buffer to be saved -- nothing is
    /// written until it is.
    pub(crate) fn start_merge(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        if self.current.as_deref() != Some(path) {
            // * The merge lands in the open buffer, so the file being merged
            //   into has to be the one on screen.
            self.open_file(path.to_string(), None, cx);
        }
        let mut candidates: Vec<String> = state
            .vault()
            .files()
            .iter()
            .map(|f| f.path.clone())
            .filter(|p| p != path && p.ends_with(".md"))
            .collect();
        candidates.sort();
        let name = path.rsplit('/').next().unwrap_or(path);
        self.open_prompt(
            PendingOp::Merge {
                file: path.to_string(),
                other: String::new(),
            },
            PromptSpec {
                title: "Merge entire file with...".into(),
                detail: Some(format!("{name} takes in another note's changes.").into()),
                input: Some(String::new()),
                placeholder: "Note to merge in".into(),
                suggestions: candidates,
                confirm: "Merge".into(),
                secondary: None,
                tertiary: None,
            },
            window,
            cx,
        );
    }

    /// Runs the merge once the chooser names its note: three-way against
    /// the two files on disk, with the shared base taken as what the open
    /// note held when it was loaded. The buffer is replaced with the result
    /// and left dirty, so the merge is reviewed before it is saved.
    pub(crate) fn finish_merge(
        &mut self,
        file: &str,
        other: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state.clone() else {
            return;
        };
        if other.trim().is_empty() {
            return;
        }
        let other = if other.ends_with(".md") {
            other.to_string()
        } else {
            format!("{other}.md")
        };
        if other == file {
            self.prompt.update(cx, |p, cx| {
                p.set_error("A note cannot be merged with itself.", cx)
            });
            return;
        }
        let theirs = match std::fs::read_to_string(state.vault().absolute(&other)) {
            Ok(text) => text,
            Err(err) => {
                let message = format!("Could not read {other}: {err}");
                self.prompt.update(cx, |p, cx| p.set_error(message, cx));
                return;
            }
        };
        // * The base is what was last saved: the disk copy, read before the
        //   buffer's own edits are folded in.
        let ours = self.editor.read(cx).file_contents();
        let base = std::fs::read_to_string(state.vault().absolute(file)).unwrap_or_default();
        let (merged, conflicts) = crate::merge::merge(&base, &ours, &theirs);
        self.pending = None;
        self.close_modal(window, cx);
        self.editor
            .update(cx, |e, cx| e.set_text(file.to_string(), merged, cx));
        match conflicts {
            0 => self.report(format!("Merged {other} into {file}; not saved yet"), cx),
            n => self.report(
                format!(
                    "Merged {other} into {file} with {n} conflict{}; not saved yet",
                    plural(n)
                ),
                cx,
            ),
        }
        cx.notify();
    }

    /// The explorer menu's "Open version history": the snapshots this note
    /// was saved through, newest first. Picking one loads those bytes into
    /// the buffer, unsaved, so the restore is reviewed before it lands --
    /// the same rule the merge follows.
    pub(crate) fn start_version_history(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let history = crate::history::History::new(&state.root, state.vault().config_dir());
        let versions = history.list(path);
        if versions.is_empty() {
            self.report(format!("{path} has no saved versions yet"), cx);
            return;
        }
        // * The stamp is the suggestion's text, so what the picker matches
        //   on is what gets looked up again on submit.
        let suggestions: Vec<String> = versions.iter().map(|v| v.label()).collect();
        let name = path.rsplit('/').next().unwrap_or(path);
        self.open_prompt(
            PendingOp::RestoreVersion {
                file: path.to_string(),
            },
            PromptSpec {
                title: format!("Version history for {name}").into(),
                detail: Some(
                    format!(
                        "{} saved version{}; the newest is first.",
                        versions.len(),
                        plural(versions.len())
                    )
                    .into(),
                ),
                input: Some(String::new()),
                placeholder: "Pick a version".into(),
                suggestions,
                confirm: "Restore".into(),
                secondary: None,
                tertiary: None,
            },
            window,
            cx,
        );
    }

    /// Loads the picked snapshot's bytes into the buffer, leaving the note
    /// dirty rather than saving over the current contents.
    pub(crate) fn finish_restore_version(
        &mut self,
        file: &str,
        label: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let history = crate::history::History::new(&state.root, state.vault().config_dir());
        let Some(version) = history.list(file).into_iter().find(|v| v.label() == label) else {
            self.prompt.update(cx, |p, cx| {
                p.set_error(format!("No version {label} any more."), cx)
            });
            return;
        };
        let Some(contents) = history.read(file, version.at) else {
            self.prompt.update(cx, |p, cx| {
                p.set_error("That version could not be read.", cx)
            });
            return;
        };
        self.pending = None;
        self.close_modal(window, cx);
        if self.current.as_deref() != Some(file) {
            self.open_file(file.to_string(), None, cx);
        }
        self.editor
            .update(cx, |e, cx| e.set_text(file.to_string(), contents, cx));
        self.report(format!("{file} restored from {label}; not saved yet"), cx);
        cx.notify();
    }

    pub(crate) fn delete_now(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let was_current = self.current.as_deref() == Some(path)
            || self
                .current
                .as_deref()
                .is_some_and(|c| c.starts_with(&format!("{path}/")));
        if was_current {
            // * Nothing may write the buffer back into a file that is gone,
            //   and no pane may go on describing it.
            self.detach_current(false, &state, cx);
            // * The main area may stop painting the view that was showing the
            //   note, and a focus left on it would sit outside the dispatch
            //   tree and swallow every key. The same rule as everywhere else
            //   decides where it goes instead: the workspace root when nothing
            //   is painted, the detached buffer when one stayed behind.
            self.focus_main(window, cx);
        }
        self.forget_history(path);
        match state.trash(path) {
            // * Obsidian shows nothing for a delete that worked.
            Ok(changes) => self.after_changes(changes, cx),
            Err(err) => self.report(err, cx),
        }
    }

    pub(crate) fn duplicate(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        match state.duplicate(path) {
            // * Nothing is reported for a copy that worked either; the new
            //   file opening is the feedback. A copied folder is reported
            //   instead, since there is nothing to open.
            Ok((copy, changes)) => {
                let is_folder = state.vault().absolute(&copy).is_dir();
                self.after_changes(changes, cx);
                if is_folder {
                    self.report(format!("Copied to {copy}"), cx);
                } else {
                    self.open_file(copy, None, cx);
                }
            }
            Err(err) => self.report(err, cx),
        }
    }

    /// Note composer's "Merge current file with another file": a prompt for
    /// the note that absorbs the current one. The prompt validates nothing
    /// by itself — the target may be a note to create, which is what
    /// Obsidian's command palette flow offers too.
    pub(crate) fn start_merge_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.current.clone() else {
            return;
        };
        self.open_prompt(
            PendingOp::MergeCurrent(source.clone()),
            PromptSpec {
                title: format!("Merge \"{}\" with...", basename(&source)).into(),
                detail: Some(
                    "The current note's content is appended to the target and every \
link to it is rewritten. The current note is deleted."
                        .into(),
                ),
                input: Some(String::new()),
                placeholder: "Target note name".into(),
                confirm: "Merge".into(),
                ..PromptSpec::default()
            },
            window,
            cx,
        );
    }

    /// The merge itself: append the source note's text under the target's,
    /// rewrite every link that resolved to the source to point at the
    /// target, then delete the source. Obsidian's own order is the same
    /// (write, rewrite links, remove), and the notice is its wording.
    pub(crate) fn merge_current(
        &mut self,
        source: &str,
        target_value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Option<String>, String> {
        let Some(state) = self.state.clone() else {
            return Ok(None);
        };
        // * The target is a note name or path as written; `new_note_path`
        //   turns a bare name into a path under the vault's new-file folder,
        //   the same place an untitled note from a wikilink would go.
        let target = if target_value.contains('/') {
            let path = format!("{target_value}.md");
            if !state.vault().absolute(&path).exists() {
                return Err(format!("{target_value} does not exist"));
            }
            path
        } else {
            let resolved = state
                .cache()
                .resolve(target_value, source)
                .map(str::to_string);
            resolved
                .filter(|p| FileKind::from_path(p).is_markdown())
                .unwrap_or_else(|| state.new_note_path(target_value, Some(source)))
        };
        if target == source {
            return Err("A note cannot merge into itself".to_string());
        }

        // * The open note's unsaved edits live in the editor buffer, not
        //   on disk or in the index: the autosave is two seconds out and
        //   a merge can land inside that window. Merging either side from
        //   disk would silently drop those edits, so the buffer wins for
        //   the note it holds.
        let open = {
            let editor = self.editor.read(cx);
            let name = editor.name.to_string();
            if name.is_empty() || !editor.is_dirty() {
                None
            } else {
                Some((name, editor.file_contents()))
            }
        };
        let read_note = |path: &str| -> Result<String, String> {
            if let Some((open_name, text)) = &open {
                if open_name == path {
                    return Ok(text.clone());
                }
            }
            state
                .texts()
                .get(path)
                .map(|t| t.to_string())
                .or_else(|| std::fs::read_to_string(state.vault().absolute(path)).ok())
                .ok_or_else(|| format!("{path} could not be read"))
        };
        let source_text = read_note(source)?;
        let target_text = read_note(&target)?;
        // * Obsidian separates the two halves with one blank line; a
        //   trailing newline on either side would otherwise stack into a
        //   triple newline that the next save would happily write back.
        let mut merged = format!(
            "{}\n\n{}",
            target_text.trim_end_matches(['\n', '\r']),
            source_text.trim_end_matches(['\n', '\r'])
        );
        if source_text.ends_with('\n') {
            merged.push('\n');
        }

        // * The link rewrites are planned against the pre-merge index and
        //   written before the source disappears, so a failure leaves the
        //   vault consistent: every note still resolves as it did.
        let plan = state
            .cache()
            .plan_merge_rewrites(source, &target, state.link_format(), &|p| {
                state.texts().get(p).map(|t| t.to_string())
            });
        for rw in &plan {
            let absolute = state
                .vault()
                .write_path(&rw.new_path)
                .map_err(|e| e.to_string())?;
            silica_vault::write_no_follow(&absolute, rw.text.as_bytes())
                .map_err(|e| format!("Could not update links in {}: {e}", rw.new_path))?;
        }

        // * The target may not exist yet (`create_note` refuses an existing
        //   path), so an existing one is written through the vault's save
        //   path and a new one through `create_note`; both index the result.
        let changes = if state.vault().absolute(&target).exists() {
            state
                .vault
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .write_bytes(&target, merged.as_bytes())
                .map_err(|e| e.to_string())?;
            self.after_changes(
                state.absorb(
                    state
                        .vault
                        .write()
                        .unwrap_or_else(|e| e.into_inner())
                        .refresh_path(&state.vault().absolute(&target)),
                ),
                cx,
            );
            None
        } else {
            Some(state.create_note(&target, &merged)?)
        };
        if let Some(changes) = changes {
            self.after_changes(changes, cx);
        }

        let was_current = self.current.as_deref() == Some(source);
        let changes = state.trash(source)?;
        if was_current {
            self.detach_current(false, &state, cx);
            self.focus_main(window, cx);
        }
        self.after_changes(changes, cx);
        self.open_file(target, None, cx);

        let links: usize = plan.iter().map(|r| r.links).sum();
        Ok((links > 0).then(|| {
            format!(
                "Updated {} link{} in {} file{}.",
                links,
                plural(links),
                plan.len(),
                plural(plan.len()),
            )
        }))
    }

    /// Enter in the prompt: finish the pending operation with `value`.
    pub(crate) fn prompt_submitted(&mut self, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(op) = self.pending.clone() else {
            self.close_modal(window, cx);
            return;
        };
        let value = value.trim();
        let watch = matches!(op, PendingOp::ExportSiteWatch);
        match op {
            PendingOp::Rename(from) => {
                // * The box holds the stem, which is what `renamed_path`
                //   takes -- the same path and the same validator the view
                //   header's inline rename field commits through.
                let to = match self.renamed_path(&from, value) {
                    Ok(to) => to,
                    Err(err) => {
                        self.prompt.update(cx, |p, cx| p.set_error(err, cx));
                        return;
                    }
                };
                self.rename_with_links(&from, &to, Some(window), cx);
            }
            PendingOp::Move(from) => {
                let folder = value.trim_matches('/');
                let name = from.rsplit('/').next().unwrap_or(&from);
                let to = if folder.is_empty() {
                    name.to_string()
                } else {
                    format!("{folder}/{name}")
                };
                self.rename_with_links(&from, &to, Some(window), cx);
            }
            PendingOp::Delete(path) => {
                self.pending = None;
                self.close_modal(window, cx);
                self.delete_now(&path, window, cx);
            }
            PendingOp::ExportSite | PendingOp::ExportSiteWatch => {
                self.pending = None;
                self.close_modal(window, cx);
                // * The export reads the vault through its own borrow, so
                //   the file list is cloned out and the state's borrow ends
                //   before the report takes `self` back.
                let (vault_root, files) = match self.state.as_ref() {
                    Some(state) => {
                        let vault = state.vault();
                        (vault.root().to_path_buf(), vault.files().to_vec())
                    }
                    None => return,
                };
                // * An empty box takes the pre-filled sibling folder back,
                //   and a relative value lands beside the vault.
                let out = if value.is_empty() {
                    None
                } else if Path::new(value).is_absolute() {
                    Some(PathBuf::from(value))
                } else {
                    vault_root.parent().map(|p| p.join(value))
                };
                let Some(out) = out.or_else(|| {
                    let name = vault_root
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "vault".into());
                    vault_root.parent().map(|p| p.join(format!("{name}-site")))
                }) else {
                    self.report("No folder to export into.", cx);
                    return;
                };
                match crate::site::export(&vault_root, &files, &out) {
                    Ok(done) => {
                        self.last_site_export = Some(done.output.clone());
                        self.report(
                            format!(
                                "Exported {} note{}, {} link{} to {}",
                                done.notes,
                                if done.notes == 1 { "" } else { "s" },
                                done.links,
                                if done.links == 1 { "" } else { "s" },
                                done.output.display()
                            ),
                            cx,
                        );
                        if watch {
                            self.toggle_site_watch(vault_root, out, cx);
                        }
                    }
                    Err(err) => self.report(err, cx),
                }
            }
            PendingOp::NewFolder { parent } => {
                let path = if parent.is_empty() {
                    value.to_string()
                } else {
                    format!("{parent}/{value}")
                };
                let result = self
                    .state
                    .as_ref()
                    .map(|s| s.create_folder(&path))
                    .unwrap_or(Ok(()));
                match result {
                    Ok(()) => {
                        self.pending = None;
                        self.close_modal(window, cx);
                        self.refresh_vault_views(cx);
                        self.explorer.update(cx, |e, cx| e.reveal_folder(&path, cx));
                        self.report(format!("Created folder {path}"), cx);
                    }
                    Err(err) => self.prompt.update(cx, |p, cx| p.set_error(err, cx)),
                }
            }
            PendingOp::UpdateLinks { from, to } => {
                self.finish_rename(&from, &to, true, Some(window), cx)
            }
            PendingOp::AddBookmark(draft) => {
                self.pending = None;
                self.close_modal(window, cx);
                self.finish_bookmark(draft, value, cx);
            }
            // * Save on "Edit bookmark" changes nothing, as Obsidian's did.
            PendingOp::EditBookmark(_) => {
                self.pending = None;
                self.close_modal(window, cx);
            }
            PendingOp::Merge { file, .. } => self.finish_merge(&file, value, window, cx),
            PendingOp::RestoreVersion { file } => {
                self.finish_restore_version(&file, value, window, cx)
            }
            PendingOp::MergeCurrent(source) => {
                self.pending = None;
                self.close_modal(window, cx);
                if value.is_empty() {
                    self.report("Merge needs a target note", cx);
                    return;
                }
                match self.merge_current(&source, value, window, cx) {
                    Ok(notice) => {
                        if let Some(notice) = notice {
                            self.report(notice, cx);
                        }
                    }
                    Err(err) => self.report(err, cx),
                }
            }
            PendingOp::RenameTag { from } => {
                self.pending = None;
                self.close_modal(window, cx);
                let Some(state) = self.state.clone() else {
                    return;
                };
                let new_tag = value.trim();
                if new_tag.is_empty() {
                    self.report("Tag name cannot be empty", cx);
                    return;
                }
                if !new_tag.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '/') {
                    self.report("Tag name contains invalid characters", cx);
                    return;
                }
                if new_tag == from {
                    return;
                }
                // * Find every file containing the old tag, rewrite it in
                //   place, and let the watcher re-index. The rename rewrites
                //   inline #tags and frontmatter tags alike.
                let tag_lower = from.to_lowercase();
                let files = state
                    .cache()
                    .tags()
                    .get(&tag_lower)
                    .map(|e| e.files.clone())
                    .unwrap_or_default();
                let vault_root = state.vault().root().to_path_buf();
                let mut changed = Vec::new();
                for path in &files {
                    let absolute = vault_root.join(path);
                    let Ok(content) = std::fs::read_to_string(&absolute) else {
                        continue;
                    };
                    let next = rename_tag_in_content(&content, &from, new_tag);
                    if next != content {
                        if let Err(err) =
                            silica_vault::write_no_follow(&absolute, next.as_bytes())
                        {
                            self.report(format!("Could not write {path}: {err}"), cx);
                            return;
                        }
                        changed.push(path.clone());
                    }
                }
                if !changed.is_empty() {
                    let count = changed.len();
                    self.after_changes(
                        Changes {
                            files: changed
                                .into_iter()
                                .map(FileChange::Modified)
                                .collect(),
                            notes: Vec::new(),
                            removed_notes: Vec::new(),
                        },
                        cx,
                    );
                    self.report(
                        format!(
                            "Renamed \"#{from}\" to \"#{new_tag}\" in {} file{}",
                            count,
                            if count == 1 { "" } else { "s" }
                        ),
                        cx,
                    );
                } else {
                    self.report("No files to rename", cx);
                }
            }
        }
    }

    /// The prompt's third button: "Always update" on the Update-links
    /// prompt, which writes `alwaysUpdateLinks: true` — so this prompt is
    /// never shown again — and then renames with the links rewritten.
    pub(crate) fn prompt_tertiary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(PendingOp::UpdateLinks { from, to }) = self.pending.clone() else {
            return;
        };
        let Some(state) = self.state.clone() else {
            return;
        };
        let mut app = state.app().clone();
        app.always_update_links = Some(true);
        if let Err(err) = state.apply_setting(&SettingChange::App(app)) {
            self.report(format!("Could not save the setting: {err}"), cx);
        }
        self.finish_rename(&from, &to, true, Some(window), cx);
    }

    /// The prompt's secondary button: "Don't update" the links.
    pub(crate) fn prompt_secondary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.pending.clone() {
            Some(PendingOp::UpdateLinks { from, to }) => {
                self.finish_rename(&from, &to, false, Some(window), cx);
            }
            Some(PendingOp::EditBookmark(path)) => {
                self.pending = None;
                self.close_modal(window, cx);
                self.change_bookmarks(cx, move |b| {
                    b.remove_last_file(&path);
                });
            }
            _ => {}
        }
    }
}
