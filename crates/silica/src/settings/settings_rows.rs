//! Settings row content: the hotkey editor and the builders for each
//! tab's rows — the methods of the split-out content sections, unchanged
//! apart from living in their own module. Split from `settings.rs` per
//! the audit's module-size finding.

use super::*;

impl SettingsView {
    /// Waits for the next chord to bind to `id`. The keyboard goes to the
    /// view's own handle so the filter field does not eat it.
    pub(crate) fn start_capture(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.capturing = Some(id);
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// The chord pressed while capturing, as the binding for that command.
    pub(crate) fn key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.capturing.clone() else {
            return;
        };
        cx.stop_propagation();
        let Some(hotkey) = hotkey_from_keystroke(&event.keystroke) else {
            return;
        };
        self.capturing = None;
        self.focus_tab(window, cx);
        self.change_hotkeys(
            cx,
            |overrides, command| {
                let list = overrides
                    .entry(command.id.clone())
                    .or_insert_with(|| command.default_hotkeys.clone());
                if !list.contains(&hotkey) {
                    list.push(hotkey);
                }
            },
            &id,
        );
    }

    /// Takes one binding off a command. A default binding taken off writes
    /// the remaining defaults as the entry, and an entry left empty unbinds.
    pub(crate) fn remove_hotkey(&mut self, id: &str, hotkey: Hotkey, cx: &mut Context<Self>) {
        self.change_hotkeys(
            cx,
            |overrides, command| {
                let list = overrides
                    .entry(command.id.clone())
                    .or_insert_with(|| command.default_hotkeys.clone());
                list.retain(|h| *h != hotkey);
            },
            id,
        );
    }

    /// Drops the command's entry, so its defaults are back.
    pub(crate) fn restore_default_hotkeys(&mut self, id: &str, cx: &mut Context<Self>) {
        self.change_hotkeys(
            cx,
            |overrides, command| {
                overrides.shift_remove(&command.id);
            },
            id,
        );
    }

    /// Reads `hotkeys.json` as it stands, lets `edit` change the entry for
    /// `id`, and asks the workspace to write it. The file's own order is
    /// kept, so a vault's bindings do not shuffle the first time Silica
    /// writes one.
    pub(crate) fn change_hotkeys(
        &mut self,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut Hotkeys, &Command),
        id: &str,
    ) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let Some(command) = self.all_commands.iter().find(|c| c.id == id).cloned() else {
            return;
        };
        let mut overrides = match state.vault().hotkeys() {
            Ok(overrides) => overrides,
            Err(err) => {
                // ! An unreadable file must not be rewritten from defaults.
                eprintln!("hotkeys.json could not be read: {err}");
                return;
            }
        };
        edit(&mut overrides, &command);
        cx.emit(SettingsEvent::Changed(Box::new(SettingChange::Hotkeys(
            overrides,
        ))));
    }

    /// How many commands each chord is bound to, so a duplicate can be
    /// marked the way Obsidian marks one.
    pub(crate) fn chord_counts(&self) -> std::collections::HashMap<String, usize> {
        let mut counts = std::collections::HashMap::new();
        for c in &self.all_commands {
            for h in &c.hotkeys {
                if let Some(k) = keystroke(h) {
                    *counts.entry(k).or_insert(0) += 1;
                }
            }
        }
        counts
    }

    /// What a click that lands anywhere but on the control itself leaves
    /// behind: nothing. Obsidian's dropdown is a native `<select>`, which no
    /// outside click can survive, and its text settings write on every
    /// keystroke, so moving on is never a way to lose what was typed.
    pub(crate) fn dismiss_open_controls(&mut self, cx: &mut Context<Self>) {
        let had_menu = self.menu.take().is_some();
        if self.editing.is_some() {
            self.commit_field(cx);
        } else if had_menu {
            cx.notify();
        }
    }

    /// Whether this click was spent closing an open dropdown, and so must
    /// not also do whatever it landed on.
    ///
    /// Obsidian's dropdown is a native `<select>`, whose option list is the
    /// platform's own window: the click that dismisses it never reaches the
    /// page, so the toggle under it does not flip. Silica's popover is part
    /// of the same window, so the rule has to be spelled out — every
    /// clickable in the modal asks this first, the popover's own option rows
    /// excepted, since a click there is the pick.
    pub(crate) fn click_dismissed_a_popover(&mut self, cx: &mut Context<Self>) -> bool {
        if !popover_swallows_click(&mut self.menu) {
            return false;
        }
        cx.notify();
        true
    }

    /// Makes the change a click on a control asks for, after the text
    /// field that click also closed has made its own.
    ///
    /// Every [`SettingChange`] carries a whole config file read from the
    /// vault as it stands, and the workspace writes the file for each one.
    /// A click on a control while a field is open changes two settings, so
    /// emitting both from the one click would build both from the config as
    /// it was before either landed, and the second write would drop the
    /// first. Committing the field now and deferring the control's own
    /// change puts that change after the field's write, which is what it has
    /// to be built on top of.
    pub(crate) fn after_open_field(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
    ) {
        if self.editing.is_some() {
            self.commit_field(cx);
            cx.defer_in(window, |this, _, cx| change(this, cx));
            return;
        }
        change(self, cx);
    }

    /// A click on a toggle: ask the workspace to write the flipped value.
    /// The modal redraws from the vault's config once the write lands, so a
    /// failed write leaves the control where it was.
    pub(crate) fn toggle_flag(&mut self, flag: Flag, window: &Window, cx: &mut Context<Self>) {
        self.after_open_field(window, cx, move |this, cx| {
            let Some(state) = this.state.clone() else {
                return;
            };
            cx.emit(SettingsEvent::Changed(Box::new(flag.toggled(&state))));
        });
    }

    pub(crate) fn refresh_hotkeys(&mut self, cx: &mut Context<Self>) {
        let query = self.hotkey_search.read(cx).text().trim().to_string();
        let filter = self.hotkey_filter;
        self.hotkey_rows = self
            .all_commands
            .iter()
            .enumerate()
            .filter(|(_, c)| filter.keeps(c))
            .filter(|(_, c)| query.is_empty() || fuzzy_match(&query, &c.name).is_some())
            .map(|(ix, _)| ix)
            .collect();
        cx.notify();
    }

    // ----- tab contents -------------------------------------------------

    pub(crate) fn items(&self, tab: Tab) -> Vec<Item> {
        let Some(state) = self.state.as_ref() else {
            return vec![item(
                "No vault is open",
                "Settings read and write a vault. Open one from the vault switcher in the left sidebar, then reopen Settings.",
                Control::None,
            )];
        };
        match tab {
            Tab::General => Self::general_items(state),
            Tab::Appearance => Self::appearance_items(state),
            Tab::Interface => Self::interface_items(state),
            Tab::Editor => Self::editor_items(state),
            Tab::Files => Self::files_items(state),
            Tab::Hotkeys => Vec::new(),
            Tab::CorePlugins => Self::core_plugin_items(state),
            Tab::CommunityPlugins => Self::community_plugin_items(state),
            Tab::Plugin(id) => Self::plugin_items(state, &self.all_commands, id),
        }
    }

    pub(crate) fn general_items(state: &VaultState) -> Vec<Item> {
        let vault = state.vault();
        vec![
            item(
                &format!("Version {}", env!("CARGO_PKG_VERSION")),
                t("Silica reads this vault the way Obsidian 1.14.4 does."),
                Control::Button(t("Check for updates").into()),
            ),
            choice(
                t("Language"),
                t("Change the display language. The Settings window's labels follow it; notes and the other views are still English."),
                state,
                LANGUAGE,
            ),
            item(
                t("Help"),
                t("Learn how to use Obsidian and get help from the community."),
                Control::Button(t("Open").into()),
            ),
            heading(t("Vault")),
            text(t("Vault name"), "", vault.name()),
            text(t("Vault folder"), "", vault.root().display().to_string()),
        ]
    }

    pub(crate) fn appearance_items(state: &VaultState) -> Vec<Item> {
        let a = &state.appearance();
        let _snippets = a.enabled_css_snippets.as_ref().map_or(0, |s| s.len());
        let snippet_dir = state
            .vault()
            .root()
            .to_path_buf()
            .join(".obsidian/snippets");
        let available_snippets = std::fs::read_dir(&snippet_dir).ok().map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "css"))
                .filter_map(|e| {
                    e.file_name()
                        .to_str()
                        .map(|s| s.trim_end_matches(".css").to_string())
                })
                .collect::<Vec<_>>()
        });
        let mut items: Vec<Item> = vec![choice(
            "Base color scheme",
            "Choose Obsidian's default color scheme.",
            state,
            BASE_COLOR_SCHEME,
        )];
        items.push(Item {
            heading: false,
            name: "Accent color".into(),
            desc: "Choose the accent color used throughout the app.".into(),
            control: Control::Color(
                state
                    .appearance()
                    .accent_color
                    .clone()
                    .unwrap_or_default()
                    .into(),
            ),
            flag: None,
            typed: None,
            number: None,
            field: Some(ACCENT_COLOR),
            honored: Honored::Live,
        });
        items.push(field(
            "Themes",
            "Manage installed themes and browse community themes.",
            state,
            CSS_THEME,
        ));
        items.push(heading("CSS snippets"));
        if let Some(snippets) = &available_snippets {
            if snippets.is_empty() {
                items.push(item(
                    "No snippets found",
                    "Add .css files to .obsidian/snippets/ in this vault.",
                    Control::None,
                ));
            } else {
                for name in snippets {
                    items.push(toggle(
                        name,
                        "Toggle this CSS snippet",
                        state
                            .appearance()
                            .enabled_css_snippets
                            .as_ref()
                            .is_some_and(|s| s.iter().any(|s| s == name)),
                        Flag::CssSnippet(name.to_string()),
                    ));
                }
            }
        } else {
            items.push(item(
                "CSS snippets folder not found",
                "Create .obsidian/snippets/ in this vault to add snippets.",
                Control::None,
            ));
        }
        items.push(heading("Font"));
        items.push(field(
            "Interface font",
            "Set base font for all of Obsidian.",
            state,
            INTERFACE_FONT_FAMILY,
        ));
        items.push(field(
            "Text font",
            "Set font for editing and reading views.",
            state,
            TEXT_FONT_FAMILY,
        ));
        items.push(field(
            "Monospace font",
            "Set font for places like code blocks and frontmatter.",
            state,
            MONOSPACE_FONT_FAMILY,
        ));
        items.push(slider(
            "Font size",
            "Font size in pixels that affects editing and reading views.",
            state,
            FONT_SIZE,
        ));
        items
    }

    pub(crate) fn interface_items(state: &VaultState) -> Vec<Item> {
        let a = &state.appearance();
        vec![
            appearance_toggle(
                "Show tab title bar",
                "Display the header at the top of every tab.",
                a,
                SHOW_VIEW_HEADER,
            ),
            appearance_toggle(
                "Show ribbon",
                "Display vertical toolbar on the side of the window.",
                a,
                SHOW_RIBBON,
            ),
            heading("Advanced"),
            appearance_toggle(
                "Native menus",
                // * Said plainly, because the answer differs by platform and
                //   a row that promised a menu bar on Linux would be lying.
                "Menus throughout the app will match the operating system. They will not be affected by your theme. macOS gets the system menu bar; Linux and Windows draw their menus in the window, so this changes nothing there.",
                a,
                NATIVE_MENUS,
            ),
            // * Obsidian keeps this one outside the vault: changing it wrote
            //   nothing under `.obsidian` (probed 2026-10-20), so it lives in
            //   the per-user store beside the vault list — the same place
            //   Obsidian's own copy is.
            choice(
                "Window frame style",
                "Determines the styling of the title bar of Obsidian windows. Requires a full restart to take effect.",
                state,
                FRAME_STYLE,
            )
            .on_restart(),
        ]
    }

    pub(crate) fn editor_items(state: &VaultState) -> Vec<Item> {
        let app = &state.app();
        let editor_status = match state.core_plugins().as_ref() {
            Some(CorePlugins::Map(m)) => m.get("editor-status").copied().unwrap_or(true),
            Some(CorePlugins::List(l)) => l.iter().any(|x| x == "editor-status"),
            None => true,
        };
        vec![
            choice(
                "Default view for new tabs",
                "The default view that a new Markdown tab gets opened in.",
                state,
                DEFAULT_VIEW_MODE,
            ),
            choice(
                "Default editing mode",
                "The default editing mode a new tab will start with.",
                state,
                LIVE_PREVIEW,
            ),
            toggle(
                "Show editing mode in status bar",
                "Show the editing mode toggle in the status bar.",
                editor_status,
                Flag::CorePlugin("editor-status", true),
            ),
            heading("Display"),
            app_toggle(
                "Inline title",
                "Display the filename as an editable title inline with the file contents.",
                app,
                SHOW_INLINE_TITLE,
            ),
            app_toggle(
                "Readable line length",
                "Limit maximum line length. Less content fits onscreen, but long blocks of text are more readable.",
                app,
                READABLE_LINE_LENGTH,
            ),
            app_toggle(
                "Strict line breaks",
                "Markdown specs ignore single line breaks in reading view. Turn this off to make single line breaks visible.",
                app,
                STRICT_LINE_BREAKS,
            ),
            choice(
                "Properties in document",
                "Choose how properties are displayed at the top of notes. Select “source” to show properties as raw YAML.",
                state,
                PROPERTIES_IN_DOCUMENT,
            ),
            app_toggle(
                "Fold heading",
                "Lets you fold all content under a heading.",
                app,
                FOLD_HEADING,
            ),
            app_toggle(
                "Fold indent",
                "Lets you fold part of an indentation, such as lists.",
                app,
                FOLD_INDENT,
            ),
            app_toggle(
                "Line numbers",
                "Show line numbers in the gutter.",
                app,
                SHOW_LINE_NUMBER,
            ),
            app_toggle(
                "Indentation guides",
                "Show vertical relationship lines between list items.",
                app,
                SHOW_INDENT_GUIDE,
            ),
            app_toggle(
                "Right-to-left (RTL)",
                "Sets the default text direction of notes to right-to-left.",
                app,
                RIGHT_TO_LEFT,
            ),
            heading("Behavior"),
            app_toggle(
                "Spellcheck",
                "Turn on the spellchecker.",
                app,
                SPELLCHECK,
            ),
            field(
                "Spellcheck languages",
                "Choose the languages for the spellchecker to use.",
                state,
                SPELLCHECK_LANGUAGES,
            ),
            app_toggle(
                "Auto-pair brackets",
                "Pair brackets and quotes automatically.",
                app,
                AUTO_PAIR_BRACKETS,
            ),
            app_toggle(
                "Auto-pair Markdown syntax",
                "Pair symbols automatically for bold, italic, code, and more.",
                app,
                AUTO_PAIR_MARKDOWN,
            ),
            app_toggle(
                "Smart lists",
                "Automatically set indentation and place list items correctly.",
                app,
                SMART_INDENT_LIST,
            ),
            app_toggle(
                "Indent using tabs",
                "Use tabs to indent by pressing the “Tab” key. Turn this off to indent using 4 spaces.",
                app,
                USE_TAB,
            ),
            slider(
                "Indent visual width",
                "Number of spaces a tab character will render as.",
                state,
                TAB_SIZE,
            ),
            heading("Advanced"),
            app_toggle(
                "Convert pasted HTML to Markdown",
                "Automatically convert HTML to Markdown when pasting and drag-and-drop from web pages.",
                app,
                AUTO_CONVERT_HTML,
            ),
            app_toggle(
                "Vim key bindings",
                "Use Vim key bindings when editing.",
                app,
                VIM_MODE,
            ),
        ]
    }

    pub(crate) fn files_items(state: &VaultState) -> Vec<Item> {
        let app = &state.app();
        let mut items = vec![
            app_toggle(
                "Always focus new tabs",
                "When you open a link in a new tab, switch to it immediately.",
                app,
                FOCUS_NEW_TAB,
            ),
            choice(
                "Default location for new notes",
                "Where newly created notes are placed.",
                state,
                NEW_FILE_LOCATION,
            ),
        ];
        // * Both location dropdowns reveal a row for the option that needs a
        //   name typed into it, and hide it for the options that do not —
        //   keyed off the selected option rather than off the stored string,
        //   so the row appears the moment the option is picked.
        //
        //   Both attachment rows are read: `editor::paste_image_attachment`
        //   and `paste_files_attachment` send every pasted file through
        //   `silica_vault::ops::attachment_folder`, which resolves all four
        //   shapes Obsidian writes.
        if NEW_FILE_LOCATION.value(state) == "folder" {
            items.push(field(
                "Folder to create new notes in",
                "Newly created notes will appear in this folder.",
                state,
                NEW_FILE_FOLDER,
            ));
        }
        items.push(choice(
            "Default location for new attachments",
            "Where newly added attachments are placed.",
            state,
            ATTACHMENT_LOCATION,
        ));
        match ATTACHMENT_LOCATION.value(state) {
            "Attachments" => items.push(
                field(
                    "Attachment folder path",
                    "Place newly created attachment files, such as images created via drag-and-drop or audio recordings, in this folder.",
                    state,
                    ATTACHMENT_FOLDER,
                ),
            ),
            "./Attachments" => items.push(
                field(
                    "Subfolder name",
                    "If your file is in “vault/folder”, and you set subfolder name to “attachments”, attachments will be saved to “vault/folder/attachments”.",
                    state,
                    SUBFOLDER_NAME,
                ),
            ),
            _ => {}
        }
        items.extend([
            heading("Links"),
            choice(
                "New link format",
                "What links to insert when auto-generating internal links.",
                state,
                NEW_LINK_FORMAT,
            ),
            app_toggle(
                "Automatically update internal links",
                "Turn off to be prompted to update links after renaming a file.",
                app,
                ALWAYS_UPDATE_LINKS,
            ),
            app_toggle(
                "Use [[Wikilinks]]",
                "Auto-generate Wikilinks for [[links]] and ![[images]] instead of Markdown links and images.",
                app,
                USE_WIKILINKS,
            ),
            app_toggle(
                "Show all file types",
                "Show files with any extension even if Obsidian can't open them natively, so you can link to them and see them in the file explorer.",
                app,
                SHOW_UNSUPPORTED_FILES,
            ),
            heading("Trash"),
            app_toggle(
                "Confirm before deleting files",
                "Avoid accidentally deleting files.",
                app,
                PROMPT_DELETE,
            ),
            choice(
                "Deleted files",
                "What happens to a file after you delete it.",
                state,
                TRASH_OPTION,
            ),
            choice(
                "Delete attachments",
                "Move attachments that no other note refers to into .trash when a note is deleted.",
                state,
                ATTACHMENT_DELETE,
            ),
            heading("Advanced"),
            field(
                "Excluded files",
                "Excluded files will be hidden in Search, Graph View, and Unlinked Mentions, less noticeable in Quick Switcher and link suggestions.",
                state,
                EXCLUDED_FILES,
            ),
            field(
                "Override config folder",
                "Use a different config folder than the default one. Must start with a dot.",
                state,
                CONFIG_FOLDER,
            )
            .on_restart(),
        ]);
        items
    }

    pub(crate) fn core_plugin_items(state: &VaultState) -> Vec<Item> {
        CORE_PLUGINS
            .iter()
            .map(|p| {
                toggle(
                    p.name,
                    p.description,
                    CorePlugins::is_enabled_or_default(state.core_plugins().as_ref(), p),
                    Flag::CorePlugin(p.id, p.default_on),
                )
                // * The toggle is read: `rebuild_commands` drops a plugin's
                //   commands from the registry, `left_pane_enabled` and
                //   `pane_enabled` close its dock panes, and `run_command`
                //   refuses it by name. What does not follow is the plugin's
                //   own internals — Silica's Files, Search and Graph are its
                //   own views rather than plugin-provided ones.
            })
            .collect()
    }

    pub(crate) fn community_plugin_items(state: &VaultState) -> Vec<Item> {
        let n = state.plugins.records().len();
        let compat = state.plugins.compat_count();
        // * Loading the runtimes belongs on this thread, and this tab is where
        //   their result is read.
        silica_plugins::live::ensure(&state.root, state.vault().config_dir(), &state.plugins);
        let mut items = vec![
            item(
                t("Community plugins"),
                t("Browse and install community plugins made by the community."),
                Control::Button(t("Browse").into()),
            ),
            item(
                t("Current plugins"),
                &match n {
                    0 => t("You currently have no plugins installed.").to_string(),
                    1 => t("You currently have 1 plugin installed.").to_string(),
                    n => format!(
                        "{} {} {}, {} {}",
                        t("You currently have"),
                        n,
                        t("plugins installed,"),
                        compat,
                        t("of which can run here.")
                    ),
                },
                Control::Button(t("Check for updates").into()),
            ),
        ];
        if n > 0 {
            items.push(heading("Installed plugins"));
            for record in state.plugins.records() {
                let m = &record.manifest;
                let mut title = m.name.clone();
                if !m.version.is_empty() {
                    title.push_str(&format!(" v{}", m.version));
                }
                if !m.author.is_empty() {
                    title.push_str(&format!(" By {}", m.author));
                }
                // * Obsidian runs whatever is installed. A native app runs
                //   the plugins that touch only the vault and its index, so
                //   each row says which tier it landed in, and a refused one
                //   gives the reason instead of a switch that would do
                //   nothing.
                let mut detail = m.description.clone();
                if !detail.is_empty() {
                    detail.push(' ');
                }
                detail.push_str(&match &record.tier {
                    Tier::Refused(refusal) => refusal.reason(),
                    tier => tier.label().to_string(),
                });
                // * What the runtimes did with it, which is the one thing a
                //   tier label cannot say: a plugin that loaded and a plugin
                //   that threw look the same until something runs one.
                let loaded = silica_plugins::live::hosts(|hosts| {
                    hosts
                        .iter()
                        .find(|host| host.id() == record.id())
                        .map(|host| host.detail())
                });
                match loaded {
                    Some(said) => {
                        if !detail.ends_with('.') {
                            detail.push('.');
                        }
                        detail.push(' ');
                        detail.push_str(&said);
                    }
                    None if record.enabled && record.tier == Tier::Compat => {
                        detail.push_str(" It is enabled but was not loaded.");
                    }
                    None => {}
                }
                items.push(
                    toggle(
                        &title,
                        &detail,
                        record.enabled,
                        Flag::CommunityPlugin(record.id().to_string()),
                    ), // * The list is Obsidian's, and the runtimes are built
                       //   when this tab or the palette is first opened: flipping
                       //   a switch here rewrites `community-plugins.json` for
                       //   the next open.
                );
            }
        }
        items
    }

    pub(crate) fn plugin_items(state: &VaultState, commands: &[Command], id: &str) -> Vec<Item> {
        match id {
            "switcher" => {
                let s = &state.switcher();
                vec![
                    switcher_toggle(
                        "Show existing only",
                        "Only show results from existing files. Links to files that are not yet created will be hidden.",
                        s,
                        SHOW_EXISTING_ONLY,
                    ),
                    switcher_toggle(
                        "Show attachments",
                        "Show attachment files like images, videos, and PDFs.",
                        s,
                        SHOW_ATTACHMENTS,
                    ),
                    switcher_toggle(
                        "Show all file types",
                        "Show all files including ones that Obsidian can't open. The file will be opened with the default app for that file type.",
                        s,
                        SHOW_ALL_FILE_TYPES,
                    ),
                ]
            }
            "command-palette" => {
                let registry = commands;
                let mut items = vec![heading("Pinned commands")];
                if state.palette.pinned.is_empty() {
                    items.push(item(
                        "No pinned commands yet",
                        "Pin a command from the command palette (Ctrl+P) to show it here.",
                        Control::None,
                    ));
                }
                for id in &state.palette.pinned {
                    let name = registry
                        .iter()
                        .find(|c| &c.id == id)
                        .map(|c| c.name.clone())
                        .unwrap_or_else(|| id.clone());
                    items.push(item(&name, id, Control::None));
                }
                items
            }
            "daily-notes" => {
                vec![
                    field(
                        "Format",
                        "The date format used for new daily notes.",
                        state,
                        TextField {
                            key: "format",
                            placeholder: DAILY_NOTE_DEFAULT_FORMAT,
                            prefix: "",
                            read: |s| s.daily_notes().format.clone().unwrap_or_default(),
                            write: |s, value| {
                                let mut next = s.daily_notes().clone();
                                if value.is_empty() {
                                    next.format = None;
                                } else {
                                    next.format = Some(value.to_string());
                                }
                                SettingChange::DailyNotes(next)
                            },
                        },
                    ),
                    field(
                        "Folder",
                        "The folder new daily notes are created in.",
                        state,
                        TextField {
                            key: "folder",
                            placeholder: "",
                            prefix: "",
                            read: |s| s.daily_notes().folder.clone().unwrap_or_default(),
                            write: |s, value| {
                                let mut next = s.daily_notes().clone();
                                if value.is_empty() {
                                    next.folder = None;
                                } else {
                                    next.folder = Some(value.to_string());
                                }
                                SettingChange::DailyNotes(next)
                            },
                        },
                    ),
                    field(
                        "Template",
                        "A template applied to new daily notes.",
                        state,
                        TextField {
                            key: "template",
                            placeholder: "",
                            prefix: "",
                            read: |s| s.daily_notes().template.clone().unwrap_or_default(),
                            write: |s, value| {
                                let mut next = s.daily_notes().clone();
                                if value.is_empty() {
                                    next.template = None;
                                } else {
                                    next.template = Some(value.to_string());
                                }
                                SettingChange::DailyNotes(next)
                            },
                        },
                    ),
                ]
            }
            "templates" => {
                vec![
                    field(
                        "Folder",
                        "The folder templates are stored in.",
                        state,
                        TextField {
                            key: "folder",
                            placeholder: "",
                            prefix: "",
                            read: |s| s.templates().folder.clone().unwrap_or_default(),
                            write: |s, value| {
                                let mut next = s.templates().clone();
                                if value.is_empty() {
                                    next.folder = None;
                                } else {
                                    next.folder = Some(value.to_string());
                                }
                                SettingChange::Templates(next)
                            },
                        },
                    ),
                    field(
                        "Date format",
                        "The moment.js format used for {{date}} in templates.",
                        state,
                        TextField {
                            key: "dateFormat",
                            placeholder: "",
                            prefix: "",
                            read: |s| s.templates().date_format.clone().unwrap_or_default(),
                            write: |s, value| {
                                let mut next = s.templates().clone();
                                if value.is_empty() {
                                    next.date_format = None;
                                } else {
                                    next.date_format = Some(value.to_string());
                                }
                                SettingChange::Templates(next)
                            },
                        },
                    ),
                    field(
                        "Time format",
                        "The moment.js format used for {{time}} in templates.",
                        state,
                        TextField {
                            key: "timeFormat",
                            placeholder: "",
                            prefix: "",
                            read: |s| s.templates().time_format.clone().unwrap_or_default(),
                            write: |s, value| {
                                let mut next = s.templates().clone();
                                if value.is_empty() {
                                    next.time_format = None;
                                } else {
                                    next.time_format = Some(value.to_string());
                                }
                                SettingChange::Templates(next)
                            },
                        },
                    ),
                ]
            }
            _ => vec![item(
                "Options not shown yet",
                "Silica does not read this plugin's settings file yet.",
                Control::None,
            )],
        }
    }
}
