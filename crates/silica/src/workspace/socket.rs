//! The control socket: the RPC methods the app exposes over its socket —
//! running commands, querying and opening files, and reading workspace
//! state — the methods of the split-out section, unchanged apart from
//! living in their own module. Split from `workspace.rs` per the audit's
//! module-size finding.

use super::*;

impl Workspace {
    /// The whole shell as JSON, for the control socket's `state` method: a
    /// test driving the app over the socket reads this instead of a
    /// screenshot. Every key is always there, `null` when there is nothing
    /// to say, so a caller can index into it without guessing. Each view
    /// describes itself; the rest is the shell's own.
    /// Teach the reading view to draw an `![[x.canvas]]` embed.
    ///
    /// The entities live here because a canvas view needs the vault; they
    /// are made on demand and reused, so a note that shows the same board
    /// twice holds one scene rather than two.
    pub(crate) fn install_canvas_host(&mut self, cx: &mut Context<Self>) {
        // * The entities are made when a note loads (see `prepare_embeds`), so
        //   this paints from a map it only reads: a paint has no `cx` to make
        //   one with.
        let canvases = self.embedded_canvases.clone();
        let host: crate::reading::view::CanvasHandler =
            std::rc::Rc::new(move |path: &str, width: f32, height: f32| {
                let found = canvases.borrow().get(path).cloned();
                match found {
                    Some(entity) => canvas_frame(entity, width, height),
                    None => crate::reading::view::missing_canvas(path, width, height),
                }
            });
        self.reading.update(cx, |r, _| r.canvas_host = Some(host));
    }

    /// Make a canvas view for every `![[x.canvas]]` the note embeds, so the
    /// handler can paint from the map without a `cx` of its own. Existing
    /// views are kept: a canvas holds a scene and a viewport, and rebuilding
    /// one on every load would throw the reader's pan away.
    pub(crate) fn prepare_embeds(&mut self, model: &reading::model::Model, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let mut wanted: Vec<String> = Vec::new();
        reading::model::each_embed(&model.blocks, &mut |embed| {
            if let reading::model::EmbedKind::Canvas(path) = &embed.kind {
                wanted.push(path.to_string_lossy().into_owned());
            }
        });
        // * Only what this note shows: anything else has been scrolled past.
        self.embedded_canvases
            .borrow_mut()
            .retain(|path, _| wanted.contains(path));
        for path in wanted {
            if self.embedded_canvases.borrow().contains_key(&path) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(state.vault().absolute(&path)) else {
                continue;
            };
            let entity = cx.new(crate::canvas::view::CanvasView::new);
            entity.update(cx, |view, cx| {
                // * At scale 1 rather than fitted: fitting the whole board into
                //   a note-width box lands below the zoom where cards keep
                //   their text, so a fitted embed draws blank cards. The
                //   reader pans and zooms it like any canvas.
                view.load(&path, &text, Some(Default::default()), state.clone(), cx)
            });
            self.embedded_canvases.borrow_mut().insert(path, entity);
        }
    }

    /// The graph as a PNG, written to `path` — the control socket's entry to
    /// the same picture the panel's *Save as image* writes.
    pub fn export_graph_png(&self, path: &std::path::Path, cx: &App) -> Result<Value, String> {
        let theme = cx.global::<Theme>().clone();
        self.graph.read(cx).export_png(path, &theme)?;
        Ok(serde_json::json!({"path": path.display().to_string()}))
    }

    pub fn describe(&self, text: bool, rows: usize, window: &Window, cx: &App) -> Value {
        let mut vault = Map::new();
        vault.insert(
            "root".into(),
            json!(self.state.as_ref().map(|s| s.root.display().to_string())),
        );
        vault.insert("name".into(), json!(self.vault_name.to_string()));
        vault.insert("loaded".into(), json!(self.state.is_some()));
        vault.insert("menu_open".into(), json!(self.vault_menu_open));
        vault.insert(
            "loading".into(),
            json!(self.loading.as_ref().map(|l| l.to_string())),
        );

        let mut file = Map::new();
        file.insert("path".into(), json!(self.current));
        file.insert(
            "detached".into(),
            json!(self.detached.as_ref().map(|n| n.to_string())),
        );
        file.insert("mode".into(), json!(self.mode.as_str()));
        file.insert("dirty".into(), json!(self.editor.read(cx).is_dirty()));
        put_capped(
            &mut file,
            "history",
            self.history.iter().map(|e| json!(e.path)).collect(),
            rows,
        );
        file.insert("history_ix".into(), json!(self.history_ix));

        let tab_json =
            |t: &Tab| json!({"path": t.path, "mode": t.mode.as_str(), "title": t.title()});
        let tabs = json!({
            "active": self.active_tab,
            "items": self.tab_snapshots().iter().map(tab_json).collect::<Vec<Value>>(),
        });
        // * The tab groups and how they are split; `tabs` above is the active
        //   group's, as it was before there could be more than one.
        let groups = json!({
            "active": self.active_group_id(),
            "layout": self.layout_tree_json(&self.layout_tree),
            "items": self
                .groups
                .iter()
                .enumerate()
                .map(|(ix, g)| {
                    let view = self.group_view(ix);
                    json!({
                        "id": g.id,
                        "active_tab": view.active_tab,
                        "dimension": g.dimension(),
                        "tabs": view.tabs.iter().map(tab_json).collect::<Vec<Value>>(),
                    })
                })
                .collect::<Vec<Value>>(),
        });

        let mut left = Map::new();
        left.insert("visible".into(), json!(self.left_visible));
        left.insert("width".into(), json!(self.left_width));
        left.insert(
            "tab".into(),
            json!(match self.left_tab {
                LeftTab::Files => "files",
                LeftTab::Search => "search",
                LeftTab::Bookmarks => "bookmarks",
            }),
        );
        left.insert("explorer".into(), self.explorer.read(cx).describe(rows));
        left.insert("search".into(), self.search.read(cx).describe(rows, cx));
        left.insert("bookmarks".into(), self.bookmarks.read(cx).describe(rows));

        let mut right = Map::new();
        right.insert("visible".into(), json!(self.right_visible));
        right.insert("width".into(), json!(self.right_width));
        right.insert(
            "tab".into(),
            json!(match self.right_tab {
                RightTab::Backlinks => "backlinks",
                RightTab::Outgoing => "outgoing",
                RightTab::Outline => "outline",
                RightTab::Footnotes => "footnotes",
                RightTab::Graph => "graph",
                RightTab::LocalGraph => "local-graph",
                RightTab::Tags => "tags",
                RightTab::Properties => "properties",
                RightTab::Calendar => "calendar",
                RightTab::FileRecovery => "file-recovery",
                RightTab::Tasks => "tasks",
                RightTab::Flashcards => "flashcards",
                RightTab::AiChat => "ai-chat",
            }),
        );
        put_capped(
            &mut right,
            "flashcards",
            self.flashcard_state
                .cards
                .iter()
                .take(rows)
                .map(|card| {
                    json!({
                        "id": card.id,
                        "source": card.source,
                        "line": card.line,
                        "kind": match &card.kind {
                            silica_flashcards::CardKind::Basic { .. } => "basic",
                            silica_flashcards::CardKind::Cloze { .. } => "cloze",
                        },
                        "due": self.flashcard_state.queue.contains(&card.id),
                        "state": self.flashcard_state.store.cards.get(&card.id),
                    })
                })
                .collect::<Vec<_>>(),
            rows,
        );
        right.insert(
            "flashcards_options".into(),
            json!({
                "current": self.flashcard_state.current,
                "show_answer": self.flashcard_state.show_answer,
                "due": self.flashcard_state.queue.len(),
                "total": self.flashcard_state.cards.len(),
            }),
        );
        right.insert(
            "ai_chat".into(),
            json!({
                "turns": self.ai_state.turns.iter().map(|turn| json!({
                    "user": turn.user,
                    "text": turn.text,
                })).collect::<Vec<_>>(),
                "sending": self.ai_state.sending,
                "error": self.ai_state.notice,
            }),
        );
        put_capped(
            &mut right,
            "outline",
            self.outline_rows(cx).iter().map(outline_json).collect(),
            rows,
        );
        right.insert(
            "outline_options".into(),
            json!({
                "file": self.outline_state.file,
                "headings": self.outline.len(),
                "follow_cursor": self.outline_state.follow_cursor,
                "show_search": self.outline_state.show_search,
                "query": self.outline_filter.read(cx).text(),
                "active": self.outline_state.active,
                "scroll": -f32::from(self.outline_state.scroll.offset().y),
            }),
        );
        put_capped(
            &mut right,
            "backlinks",
            self.pane_rows_json(RightTab::Backlinks),
            rows,
        );
        right.insert(
            "backlinks_options".into(),
            json!({
                "collapse_all": self.backlink_state.collapse_all,
                "extra_context": self.backlink_state.extra_context,
                "sort": self.backlink_state.sort.as_str(),
                "show_search": self.backlink_state.show_search,
                "query": self.backlink_filter.read(cx).text(),
            }),
        );
        put_capped(
            &mut right,
            "outgoing",
            self.pane_rows_json(RightTab::Outgoing),
            rows,
        );
        put_capped(
            &mut right,
            "tags",
            self.tag_rows(cx).iter().map(tag_json).collect(),
            rows,
        );
        right.insert(
            "tags_options".into(),
            json!({
                "sort": self.tag_state.sort.as_str(),
                "use_hierarchy": self.tag_state.use_hierarchy,
                "show_search": self.tag_state.show_search,
                "query": self.tag_filter.read(cx).text(),
                "collapsed": self.tag_state.collapsed.len(),
            }),
        );
        put_capped(
            &mut right,
            "properties",
            self.property_rows(cx)
                .iter()
                .map(|r| {
                    json!({
                        "name": r.name,
                        "count": r.count,
                        "type": r.ty.map_or("unknown", |t| t.as_str()),
                        "icon": r.icon,
                    })
                })
                .collect(),
            rows,
        );
        right.insert(
            "properties_options".into(),
            json!({
                "sort": self.property_state.sort.as_str(),
                "show_search": self.property_state.show_search,
                "query": self.property_filter.read(cx).text(),
            }),
        );

        // * Whether the graph is on screen is the shell's to say; the view
        //   itself only knows what it would draw.
        let mut graph = self.graph.read(cx).describe();
        if let Some(object) = graph.as_object_mut() {
            object.insert("open".into(), json!(matches!(self.mode, ViewMode::Graph)));
        }

        json!({
            "vault": Value::Object(vault),
            "file": Value::Object(file),
            "tabs": tabs,
            "groups": groups,
            "docks": {"left": Value::Object(left), "right": Value::Object(right)},
            "modal": self.describe_modal(rows, cx),
            "editor": self.editor.read(cx).describe(text, cx),
            "reading": self.reading.read(cx).describe(),
            "graph": graph,
            "canvas": self.canvas.read(cx).describe(cx),
            "bases": self.bases.read(cx).state_json(),
            "pdf": self.pdf.read(cx).state_json(cx),
            "video": self.reading.read(cx).video_state_json(),
            "embedded_canvases": self.embedded_canvases.borrow().keys().cloned().collect::<Vec<_>>(),
            "status": self.status_text(),
            "status_bar": self.status_bar_text(cx),
            "status_bar_items": self.status_bar_items(cx),
            // * The empty tab's pills, by the text they carry, while that is
            //   what the main area shows.
            "empty_tab": (self.current.is_none() && self.main_view(cx) == MainView::Empty).then(|| {
                empty_tab_actions(&cx.global::<Commands>().0)
                    .iter()
                    .map(EmptyAction::text)
                    .collect::<Vec<String>>()
            }),
            // * The view header's title while it is being renamed in place,
            //   so a driver can see the typed name before Enter commits it.
            "view_title": self.title_edit.as_ref().map(|edit| json!({
                "group": edit.group,
                "file": edit.path,
                "text": edit.field.text(),
                "cursor": edit.field.cursor(),
            })),
            "notices": self.notices.texts(),
            "focus": {
                "name": self.focus_name(window, cx),
                "context_stack": context_names(window),
            },
            "window": {
                "width": window.viewport_size().width.as_f32(),
                "height": window.viewport_size().height.as_f32(),
                "scale": window.scale_factor(),
                "active": window.is_window_active(),
                "title": self.window_title(),
            },
        })
    }

    /// The overlay that is up, if any: its kind plus whatever the modal
    /// itself says about its query and rows.
    pub(crate) fn describe_modal(&self, rows: usize, cx: &App) -> Value {
        let mut modal = Map::new();
        modal.insert(
            "kind".into(),
            json!(match self.modal {
                Modal::None => "none",
                Modal::Switcher => "switcher",
                Modal::Palette => "palette",
                Modal::Settings => "settings",
                Modal::Prompt => "prompt",
                Modal::CopyResults => "copy-results",
                Modal::Chooser => "chooser",
            }),
        );
        // * The four list modals fill these in; with no modal up they stay
        //   empty rather than disappearing.
        modal.insert("query".into(), Value::Null);
        modal.insert("selected".into(), Value::Null);
        modal.insert("error".into(), Value::Null);
        modal.insert("rows".into(), Value::Array(Vec::new()));
        let view = match self.modal {
            Modal::Switcher => Some(self.switcher.read(cx).describe(rows, cx)),
            Modal::Palette => Some(self.palette.read(cx).describe(rows, cx)),
            Modal::Prompt => Some(self.prompt.read(cx).describe(rows, cx)),
            Modal::CopyResults => Some(self.copy_results.read(cx).describe()),
            Modal::Chooser => Some(self.chooser.read(cx).describe(rows, cx)),
            Modal::Settings => Some(self.settings.read(cx).describe(rows, cx)),
            Modal::None => None,
        };
        if let Some(Value::Object(view)) = view {
            for (key, value) in view {
                modal.insert(key, value);
            }
        }
        // * Settings has tabs rather than rows; the tab it is on is the part
        //   a caller steers by.
        modal.insert(
            "settings_tab".into(),
            modal.get("tab").cloned().unwrap_or(Value::Null),
        );
        modal.insert(
            "pending_op".into(),
            match &self.pending {
                None => Value::Null,
                Some(PendingOp::Rename(path)) => json!({"op": "rename", "path": path}),
                Some(PendingOp::Move(path)) => json!({"op": "move", "path": path}),
                Some(PendingOp::Delete(path)) => json!({"op": "delete", "path": path}),
                Some(PendingOp::NewFolder { parent }) => {
                    json!({"op": "new-folder", "parent": parent})
                }
                Some(PendingOp::UpdateLinks { from, to }) => {
                    json!({"op": "update-links", "from": from, "to": to})
                }
                Some(PendingOp::AddBookmark(draft)) => {
                    json!({"op": "add-bookmark", "placeholder": draft.placeholder()})
                }
                Some(PendingOp::EditBookmark(path)) => {
                    json!({"op": "edit-bookmark", "path": path})
                }
                Some(PendingOp::Merge { file, .. }) => {
                    json!({"op": "merge", "path": file})
                }
                Some(PendingOp::RestoreVersion { file }) => {
                    json!({"op": "restore-version", "path": file})
                }
                Some(PendingOp::MergeCurrent(source)) => {
                    json!({"op": "merge-current", "source": source})
                }
                Some(PendingOp::RenameTag { from }) => {
                    json!({"op": "rename-tag", "from": from})
                }
                Some(PendingOp::RenameProperty { from }) => {
                    json!({"op": "rename-property", "from": from})
                }
                Some(PendingOp::SetPropertyType { key }) => {
                    json!({"op": "set-property-type", "key": key})
                }
                Some(PendingOp::DeleteProperty { key }) => {
                    json!({"op": "delete-property", "key": key})
                }
                Some(PendingOp::ExportSite) => json!({"op": "export-site"}),
                Some(PendingOp::AddAlias) => json!({"op": "add-alias"}),
                Some(PendingOp::RenameHeading { line, level }) => {
                    json!({"op": "rename-heading", "line": line, "level": level})
                },
                Some(PendingOp::ExtractHeading { source, start, end, heading }) => {
                    json!({"op": "extract-heading", "source": source, "start": start, "end": end, "heading": heading})
                },
                Some(PendingOp::SplitFile { source, line }) => {
                    json!({"op": "split-file", "source": source, "line": line})
                }
                Some(PendingOp::DeleteAttachments { path }) => {
                    json!({"op": "delete-attachments", "path": path})
                }
                Some(PendingOp::SaveWorkspace) => json!({"op": "save-workspace"}),
                Some(PendingOp::LoadWorkspace) => json!({"op": "load-workspace"}),
                Some(PendingOp::DeleteWorkspace) => json!({"op": "delete-workspace"}),
            },
        );
        Value::Object(modal)
    }

    /// Where typed text would land: "editor", "input" or "none". Deciding
    /// is separate from doing because only one of the two routes may run
    /// with this workspace borrowed: delivering a keystroke to a focused
    /// input re-enters the focused view, and that view can be this one.
    pub fn type_target(&self, window: &Window, cx: &App) -> &'static str {
        Self::typed_text_target(self.focus_name(window, cx), self.main_view(cx))
    }

    /// Which of the two routes `text` takes, for the focus the shell
    /// reports and the view the main area is painting.
    ///
    /// The editor is answered only while it is on screen. Its handle can
    /// keep the focus after the main area has stopped painting it — the
    /// keyboard reaches nothing from there, but this route does not go
    /// through the dispatch tree, and would otherwise fill a buffer that
    /// belongs to no file with text nothing ever saves.
    pub(crate) fn typed_text_target(focus: &str, painted: MainView) -> &'static str {
        match focus {
            // * A property row or a table cell is the editor's own field.
            "editor" | "property-field" | "cell-field" => match painted {
                MainView::Editor => "editor",
                _ => "none",
            },
            "canvas-editor" => match painted {
                MainView::Canvas => "canvas-editor",
                _ => "none",
            },
            "workspace" | "reading" | "graph" | "canvas" | "bases" | "pdf" | "none" => "none",
            _ => "input",
        }
    }

    /// Types `text` into the editor, for the socket's `type` method, and
    /// says so. Only the editor is reachable from here: the inputs belong
    /// to the views that own them and are typed into from outside this
    /// borrow, so a caller asks [`Workspace::type_target`] first and
    /// anything but "editor" is answered the other way.
    pub fn type_into_focused(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> &'static str {
        match self.type_target(window, cx) {
            "editor" => {
                // * `type_text` already routes to an open property row or
                //   table cell rather than the body.
                self.editor
                    .update(cx, |e, cx| e.type_text(text, window, cx));
                "editor"
            }
            "canvas-editor" => {
                if let Some(editor) = self.canvas.read(cx).card_editor().cloned() {
                    editor.update(cx, |e, cx| e.type_text(text, window, cx));
                }
                "canvas-editor"
            }
            // * Nothing else is reachable from here, so nothing else is
            //   typed into: a focused input is answered by the caller, which
            //   asked where the text would go before handing it over.
            _ => "none",
        }
    }

    /// A name for whatever has focus, for the state dump and to decide
    /// where typed text goes. Only handles this shell can reach are named
    /// by handle; the find panel's two fields belong to the editor, which
    /// is asked, and the Settings fields are named from the dispatch path.
    /// `explorer` never comes back: the file tree's rows are painted
    /// without a focus handle, so the keyboard is never in it.
    pub fn focus_name(&self, window: &Window, cx: &App) -> &'static str {
        if window.focused(cx).is_none() {
            return "none";
        }
        if self.editor.read(cx).focus_handle_ref().is_focused(window) {
            // * An open property row or table cell takes the editor's keys
            //   without taking the focus.
            return match self.editor.read(cx).field_kind() {
                Some("cell") => "cell-field",
                Some(_) => "property-field",
                None => "editor",
            };
        }
        if self.reading.focus_handle(cx).is_focused(window) {
            return "reading";
        }
        if self.graph.focus_handle(cx).is_focused(window) {
            return "graph";
        }
        if self
            .canvas
            .read(cx)
            .card_editor()
            .is_some_and(|e| e.read(cx).focus_handle_ref().is_focused(window))
        {
            return "canvas-editor";
        }
        if self.canvas.focus_handle(cx).is_focused(window) {
            return "canvas";
        }
        if self.bases.focus_handle(cx).is_focused(window) {
            return "bases";
        }
        if let Some(field) = self.pdf.read(cx).focused_field(window, cx) {
            return field;
        }
        if self.pdf.focus_handle(cx).is_focused(window) {
            return "pdf";
        }
        // * The view header's inline rename field, which takes typed text
        //   through the workspace's own input handler.
        if self.title_focus.is_focused(window) {
            return "view-title";
        }
        // * A tree row being renamed takes typed text through the
        //   explorer's own input handler.
        if self.explorer.read(cx).renaming_focus().is_focused(window) {
            return "explorer-rename";
        }
        if self.switcher.focus_handle(cx).is_focused(window) {
            return "switcher-input";
        }
        if self.palette.focus_handle(cx).is_focused(window) {
            return "palette-input";
        }
        if self.prompt.focus_handle(cx).is_focused(window) {
            return "prompt-input";
        }
        if self.chooser.focus_handle(cx).is_focused(window) {
            return "chooser-input";
        }
        if self.search.focus_handle(cx).is_focused(window) {
            return "search-input";
        }
        for (input, name) in [
            (&self.backlink_filter, "backlinks-filter"),
            (&self.tag_filter, "tags-filter"),
            (&self.property_filter, "properties-filter"),
            (&self.outline_filter, "outline-filter"),
        ] {
            if input.focus_handle(cx).is_focused(window) {
                return name;
            }
        }
        if self.settings.focus_handle(cx).is_focused(window) {
            return "settings";
        }
        if self.explorer.focus_handle(cx).is_focused(window) {
            return "explorer";
        }
        if self.focus_handle.is_focused(window) {
            return "workspace";
        }
        // * Both find fields sit in one key context, so the stack cannot tell
        //   them apart; the editor holds the two handles and can.
        if let Some(name) = self.editor.read(cx).find_focus(window, cx) {
            return name;
        }
        let stack = context_names(window);
        if stack.iter().any(|c| c == "Settings") {
            return "settings";
        }
        "unknown"
    }

    /// Focuses one of the main views by name, for the socket's `focus`
    /// method. The mode is left alone: focus is all that is asked for.
    pub fn focus_view(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        // * The tree is in the left dock, not the main area: showing the
        //   dock's Files tab is what puts its handle on screen.
        if name == "explorer" {
            self.leave_search_field(window, cx);
            self.left_visible = true;
            self.left_tab = LeftTab::Files;
            self.explorer.update(cx, |e, cx| e.focus_tree(window, cx));
            cx.notify();
            return Ok(());
        }
        match Self::focus_target(name, self.main_view(cx))? {
            // * The root div paints in every frame, so its handle is always
            //   somewhere the keyboard can sit.
            None => window.focus(&self.focus_handle, cx),
            Some(_) => self.focus_main(window, cx),
        }
        cx.notify();
        Ok(())
    }

    /// Which view the socket's `focus` names: `None` for the workspace
    /// root, or the main view asked for once it is the one being painted.
    ///
    /// A view the main area is not painting is refused rather than focused.
    /// Its handle is in no frame's dispatch tree, so the keyboard parked on
    /// it reaches nothing at all — not even the hotkey that would undo it,
    /// which is the dead state the mode switches were hardened against.
    pub(crate) fn focus_target(name: &str, painted: MainView) -> Result<Option<MainView>, String> {
        let wanted = match name {
            "workspace" => return Ok(None),
            "editor" => MainView::Editor,
            "reading" => MainView::Reading,
            "graph" => MainView::Graph,
            "canvas" => MainView::Canvas,
            "bases" => MainView::Bases,
            "pdf" => MainView::Pdf,
            other => return Err(format!("unknown view: {other}")),
        };
        if wanted != painted {
            return Err(format!(
                "the main area is painting {}, not {name}: \
                 switch the mode first, or focus the view that is on screen",
                painted.as_str()
            ));
        }
        Ok(Some(wanted))
    }

    /// Empties the status bar and takes down every notice, so a caller
    /// can tell the next notice from the one already there.
    pub fn clear_status(&mut self, cx: &mut Context<Self>) {
        self.notices.clear();
        self.status = "".into();
        cx.notify();
    }

    /// The status bar's items, left to right, for what a leaf holds.
    ///
    /// Obsidian's default status bar is right-aligned only and holds the
    /// Backlinks item and the Word count item, in that order. Both belong
    /// to the open Markdown view, so a canvas tab and a base tab show
    /// neither. An **empty** tab is not a blank status bar, though: with
    /// one active Obsidian's reads `0 properties`, `0 words`,
    /// `0 characters` (see [[Specification#The empty tab]]), so the items
    /// stay and read as zero. The caret position is Silica's own and sits
    /// in front of them; the parse and shape numbers behind it are
    /// diagnostics and appear only with `SILICA_DEBUG_STATUS` set.
    pub(crate) fn status_items(
        current: Option<&str>,
        caret: &str,
        backlinks: usize,
        words: usize,
        chars: usize,
    ) -> Vec<String> {
        let Some(path) = current else {
            return EMPTY_TAB_STATUS
                .iter()
                .map(|item| item.to_string())
                .collect();
        };
        if matches!(FileKind::from_path(path), FileKind::Canvas | FileKind::Base) {
            return Vec::new();
        }
        vec![format!(
            "{caret}{backlinks} backlink{}  ·  {words} words, {chars} characters",
            plural(backlinks)
        )]
    }

    /// The items [`Workspace::status_items`] gives the open leaf.
    pub fn status_bar_items(&self, cx: &App) -> Vec<String> {
        let (caret, words, chars) = if self.mode.is_editing() {
            let editor = self.editor.read(cx);
            let cursor = editor.cursor();
            let cursors = match editor.cursor_count() {
                1 => String::new(),
                n => format!("{n} cursors  ·  "),
            };
            let timings = if debug_status() {
                format!(
                    "parse {:.1} ms, {} spans  ·  shape {:.2} ms  ·  ",
                    editor.stats.parse_ms,
                    editor.stats.spans,
                    editor.prepaint_ms()
                )
            } else {
                String::new()
            };
            (
                format!(
                    "Ln {}, Col {}  ·  {cursors}{timings}",
                    cursor.line + 1,
                    cursor.col + 1
                ),
                editor.stats.words,
                editor.stats.chars,
            )
        } else {
            (String::new(), self.words, self.chars)
        };
        let backlinks = self
            .backlink_sections
            .first()
            .map(|section| section.count)
            .unwrap_or(0);
        Self::status_items(self.current.as_deref(), &caret, backlinks, words, chars)
    }

    /// The same items as one string, for the socket's state dump: the
    /// separator is the one the bar itself puts between them.
    pub fn status_bar_text(&self, cx: &App) -> Option<String> {
        let items = self.status_bar_items(cx);
        (!items.is_empty()).then(|| items.join("  ·  "))
    }

    /// The title the desktop shows, as the last paint set it: empty until
    /// the first frame has run.
    pub fn window_title(&self) -> SharedString {
        self.window_title.clone()
    }

    /// What the last command said, which is how every command reports:
    /// the newest notice while one is up, the status bar otherwise.
    pub fn status_text(&self) -> String {
        match self.notices.latest() {
            Some(text) => text.to_string(),
            None => self.status.to_string(),
        }
    }

    /// The open vault's folder, if one is open.
    pub fn vault_root(&self) -> Option<PathBuf> {
        self.state.as_ref().map(|s| s.root.clone())
    }

    /// An absolute path for a vault-relative one, refusing anything that
    /// points outside the vault. This is the only path the socket reads and
    /// writes through, and the vault folder is the only thing Silica may
    /// touch.
    pub fn vault_path(&self, path: &str) -> Result<PathBuf, String> {
        let root = self
            .vault_root()
            .ok_or_else(|| "No vault is open".to_string())?;
        vault_path_in(&root, path)
    }

    /// The vault's own spelling of a vault-relative path, if the index
    /// carries one: that spelling is the key every pane is looked up by, so
    /// the caller's casing is only a request for it.
    pub fn vault_file_path(&self, path: &str) -> Option<String> {
        if let Some(state) = self.state.as_ref() {
            if let Some(found) = state
                .vault()
                .files()
                .iter()
                .find(|f| f.path.eq_ignore_ascii_case(path))
            {
                return Some(found.path.clone());
            }
        }
        // ! The index fills on a background thread; while it loads, a path
        //   that exists on disk still opens instead of answering as
        //   missing. `vault_path_in` keeps the path inside the vault, so
        //   this reads existence, not trust.
        let root = self
            .state
            .as_ref()
            .map(|s| s.root.clone())
            .or_else(|| self.opening_root.clone())?;
        vault_path_in(&root, path)
            .ok()
            .filter(|p| p.is_file())
            .map(|_| path.to_string())
    }

    /// The open vault's files, for the socket's `list_files`: the same list
    /// the explorer is built from, under `folder` when one is given.
    pub fn list_vault_files(&self, folder: Option<&str>) -> Vec<Value> {
        let Some(state) = self.state.as_ref() else {
            return Vec::new();
        };
        // * Vault paths keep their case and compare without it, so a folder
        //   stored as `Sub` answers to `sub` as well.
        let prefix = folder
            .map(|f| f.trim_matches('/'))
            .filter(|f| !f.is_empty())
            .map(|f| format!("{}/", f.to_lowercase()));
        state
            .vault()
            .files()
            .iter()
            .filter(|f| {
                prefix
                    .as_deref()
                    .is_none_or(|p| f.path.to_lowercase().starts_with(p))
            })
            .map(|f| json!({"path": f.path, "kind": kind_name(f.kind), "size": f.size}))
            .collect()
    }

    /// Every command in the registry with the hotkeys `hotkeys.json` leaves
    /// it, and whether [`Workspace::run_command`] would run it *now*.
    ///
    /// `runnable` is the check callback's answer, so it moves with the
    /// window: an editing command reads false from a reading leaf, which
    /// is the same thing the palette shows.
    pub fn list_commands(&self, window: &Window, cx: &App) -> Vec<Value> {
        cx.global::<Commands>()
            .0
            .iter()
            .map(|c| {
                let hotkeys: Vec<String> = c.hotkeys.iter().filter_map(keystroke).collect();
                json!({
                    "id": c.id,
                    "name": c.name,
                    "hotkeys": hotkeys,
                    "runnable": self.can_run(&c.id, window, cx),
                })
            })
            .collect()
    }

    /// The mode a socket request names, if it names one at all.
    pub fn open_mode(&self, mode: Option<&str>) -> Result<Option<ViewMode>, String> {
        match mode {
            None => Ok(None),
            Some(name) => ViewMode::parse(name)
                .map(Some)
                .ok_or_else(|| format!("unknown mode: {name}")),
        }
    }
}
