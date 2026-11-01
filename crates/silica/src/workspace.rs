//! The workspace shell: left dock (Files, Search), main area (reading view
//! or source editor), right dock (Backlinks, Outgoing links, Outline, Tags),
//! view header, status bar, and the quick switcher, command palette and
//! Settings overlays. Commands run by id ([`Workspace::run_command`]);
//! hotkeys come from the command registry, so `hotkeys.json` applies.
//!
//! The vault is indexed on a background thread when the window opens and
//! then kept current by a filesystem watcher: changed paths are re-read and
//! re-indexed in place, link edges re-resolved, and every pane refreshed.

use crate::assets::{icon, logo};
use crate::backlinks::{self, Group, Match as PaneMatch, Section};
use crate::bases::{BasesEvent, BasesView};
use crate::bookmarks::{BookmarkEvent, BookmarksPane};
use crate::canvas::menu::tooltip;
use crate::canvas::{CanvasEvent, CanvasView};
use crate::chooser::{Chooser, ChooserEvent, Row as ChooserRow};
use crate::copy_results::{CopyResults, CopyResultsEvent};
use crate::editor::{Editor, EditorEvent};
use crate::explorer::{display_name, kind_name, ExplorerEvent, FileExplorer, FileTree, SortOrder};
use crate::fold;
use crate::graph::{GraphEvent, GraphView};
use crate::help::HelpWindow;
use crate::mentions::{Finder, Name};
use crate::palette::{CommandPalette, PaletteCommand, PaletteEvent};
use crate::panes::{self, PaneSort};
use crate::pdf::view::{PdfEvent, PdfView};
use crate::prompt::{Prompt, PromptEvent, PromptSpec};
use crate::reading::{self, LinkTarget, ReadingEvent, ReadingView, TaskMarker, TaskToggled};
use crate::search::{CopyEntry, SearchEvent, SearchPane};
use crate::settings::{self, SettingChange, SettingsEvent, SettingsView};
use crate::switcher::{QuickSwitcher, SwitcherEvent};
use crate::text_input::{self, InputEvent, TextInput};
use crate::theme::{
    load_css_snippets, load_css_theme, Theme, FONT_UI_SMALL, FONT_UI_SMALLER, HEADER_HEIGHT,
    ICON_SIZE, RIBBON_WIDTH, SIDEBAR_WIDTH, TAB_CURVE,
};
use crate::title_field::{self, TitleField};
use crate::vaults::VaultManager;
use crate::watch;
use gpui::{
    actions, anchored, canvas, deferred, div, fill, point, prelude::*, px, relative, rgb, rgba,
    size, transparent_black, Action, Anchor, App, Bounds, BoxShadow, ClickEvent, ClipboardItem,
    Context, CursorStyle, Decorations, DragMoveEvent, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Focusable, FontWeight, Global, HighlightStyle, HitboxBehavior,
    Hsla, KeyBinding, MouseButton, MouseDownEvent, PathBuilder, Pixels, ResizeEdge, ScrollHandle,
    ShapedLine, SharedString, StyledText, Subscription, Task, TextAlign, Tiling, UTF16Selection,
    Window, WindowAppearance, WindowControlArea,
};
use rayon::prelude::*;
use serde_json::{json, Map, Value};
use silica_cache::{
    basename, frontmatter_aliases, index_markdown, CachedMetadata, LinkFormat, MetadataCache,
    PropertyEntry,
};
use silica_commands::{hotkey_label, keystroke, CommandRegistry};
use silica_plugins::native::PluginContext;
use silica_formats::bookmarks::{Bookmark, Bookmarks};
use silica_formats::config::{
    self as config, AppConfig, AppearanceConfig, CommandPaletteConfig, CorePlugins,
    DailyNotesConfig, Hotkeys, PropertyTypes, SwitcherConfig, TemplatesConfig,
    CORE_PLUGINS,
};
use silica_formats::properties::{Change, PropertiesInDocument};
use silica_formats::{validate_renamed_file, FileKind};
use silica_markdown::Subpath;
use silica_templates as templates;
use silica_text::table::TableCommand;
use silica_text::{BlockInsert, LinePrefix, LinkSyntax};
use silica_vault::{FileChange, TrashOption, Vault, VaultError, VaultRegistry};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, RwLock, RwLockReadGuard};
use std::time::{Duration, Instant};

actions!(
    workspace,
    [ToggleMode, GoBack, GoForward, OpenSearch, OpenGraph]
);

/// Runs the command with this Obsidian id (bound from the registry's hotkeys).
#[derive(Debug, Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct RunCommand {
    pub id: String,
}

/// The command registry with the vault's `hotkeys.json` applied.
pub struct Commands(pub Arc<CommandRegistry>);

impl Global for Commands {}

/// The local civil time, which is what Obsidian's dates are in.
/// The clock as Obsidian stamps a bookmark: Unix milliseconds.
fn now_millis() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn now() -> templates::Stamp {
    let local = chrono::Local::now();
    let naive = local.naive_local();
    templates::Stamp {
        year: chrono::Datelike::year(&naive) as i64,
        month: chrono::Datelike::month(&naive),
        day: chrono::Datelike::day(&naive),
        hour: chrono::Timelike::hour(&naive),
        minute: chrono::Timelike::minute(&naive),
        second: chrono::Timelike::second(&naive),
    }
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.is_empty())
}

/// The ids that need the note itself: an editor on screen with the
/// keyboard in it, which is Obsidian's `editorCallback`. Such a command is
/// filtered out of the palette when there is nothing to run it in, rather
/// than listed and refused.
///
/// Every id here is guarded by [`Workspace::editing_or_report`] in
/// `run_command` as well, which is what answers the hotkey path: a palette
/// filter alone would leave the key silently inert.
pub const EDITOR_ONLY: &[&str] = &[
    "bases:insert",
    "bookmarks:bookmark-current-heading",
    "bookmarks:bookmark-current-section",
    "editor:add-cursor-above",
    "editor:add-cursor-below",
    "editor:clear-formatting",
    "editor:cycle-list-checklist",
    "editor:delete-paragraph",
    "editor:fold-all",
    "editor:fold-less",
    "editor:fold-more",
    "editor:follow-link",
    "editor:indent-list",
    "editor:insert-callout",
    "editor:insert-codeblock",
    "editor:insert-embed",
    "editor:insert-footnote",
    "editor:insert-horizontal-rule",
    "editor:insert-link",
    "editor:insert-mathblock",
    "editor:insert-table",
    "editor:insert-tag",
    "editor:insert-wikilink",
    "editor:open-link-in-new-leaf",
    "editor:open-link-in-new-split",
    "editor:set-heading",
    "editor:set-heading-0",
    "editor:set-heading-1",
    "editor:set-heading-2",
    "editor:set-heading-3",
    "editor:set-heading-4",
    "editor:set-heading-5",
    "editor:set-heading-6",
    "editor:swap-line-down",
    "editor:swap-line-up",
    "editor:toggle-blockquote",
    "editor:toggle-bold",
    "editor:toggle-bullet-list",
    "editor:toggle-checklist-status",
    "editor:toggle-code",
    "editor:toggle-comments",
    "editor:toggle-fold",
    "editor:toggle-highlight",
    "editor:toggle-highlight-blue",
    "editor:toggle-highlight-green",
    "editor:toggle-highlight-orange",
    "editor:toggle-highlight-purple",
    "editor:toggle-highlight-red",
    "editor:toggle-highlight-yellow",
    "editor:toggle-inline-math",
    "editor:toggle-italics",
    "editor:toggle-numbered-list",
    "editor:toggle-strikethrough",
    "editor:unfold-all",
    "editor:unindent-list",
    "insert-current-date",
    "insert-current-time",
    "insert-template",
];

/// The ids that need a base on screen, which is Obsidian's `checkCallback`
/// for them: with no base showing each is filtered out of the palette
/// rather than listed and refused, and asked for by id it does nothing.
///
/// `bases:insert` is not one of them — it runs in a note, not in a base, and
/// is in [`EDITOR_ONLY`] instead.
pub const BASE_ONLY: &[&str] = &[
    "bases:add-item",
    "bases:add-view",
    "bases:change-view",
    "bases:copy-table",
];

/// Command ids the workspace can run today; the palette lists only these,
/// the way Obsidian hides commands whose check callback fails.
pub const RUNNABLE: &[&str] = &[
    "command-palette:open",
    "editor:save-file",
    "editor:toggle-source",
    "editor:open-search",
    "editor:open-search-replace",
    "editor:add-cursor-above",
    "editor:add-cursor-below",
    "editor:toggle-bold",
    "editor:toggle-italics",
    "editor:toggle-strikethrough",
    "editor:toggle-highlight",
    "editor:toggle-code",
    "editor:toggle-inline-math",
    "editor:toggle-comments",
    "editor:toggle-blockquote",
    "editor:toggle-bullet-list",
    "editor:toggle-numbered-list",
    "editor:toggle-checklist-status",
    "editor:cycle-list-checklist",
    "editor:toggle-highlight-red",
    "editor:toggle-highlight-orange",
    "editor:toggle-highlight-yellow",
    "editor:toggle-highlight-green",
    "editor:toggle-highlight-blue",
    "editor:toggle-highlight-purple",
    "editor:clear-formatting",
    "editor:insert-callout",
    "editor:insert-codeblock",
    "editor:insert-mathblock",
    "editor:insert-horizontal-rule",
    "editor:insert-tag",
    "editor:insert-footnote",
    "editor:set-heading",
    "editor:toggle-fold",
    "editor:fold-all",
    "editor:unfold-all",
    "editor:fold-more",
    "editor:fold-less",
    "editor:set-heading-0",
    "editor:set-heading-1",
    "editor:set-heading-2",
    "editor:set-heading-3",
    "editor:set-heading-4",
    "editor:set-heading-5",
    "editor:set-heading-6",
    "editor:indent-list",
    "editor:unindent-list",
    "editor:delete-paragraph",
    "editor:swap-line-up",
    "editor:swap-line-down",
    "editor:insert-link",
    "editor:insert-wikilink",
    "editor:insert-embed",
    "editor:follow-link",
    "editor:open-link-in-new-leaf",
    "switcher:open",
    "global-search:open",
    "graph:open",
    "graph:open-local",
    "canvas:new-file",
    "canvas:jump-to-group",
    "canvas:convert-to-file",
    "bases:new-file",
    "bases:add-view",
    "bases:insert",
    "bases:copy-table",
    "bases:change-view",
    "bases:add-item",
    "markdown:toggle-preview",
    "markdown:add-metadata-property",
    "markdown:clear-metadata-properties",
    "editor:toggle-fold-properties",
    "editor:insert-table",
    "insert-template",
    "insert-current-date",
    "insert-current-time",
    "daily-notes",
    "daily-notes:goto-prev",
    "daily-notes:goto-next",
    "editor:table-row-before",
    "editor:table-row-after",
    "editor:table-row-up",
    "editor:table-row-down",
    "editor:table-row-copy",
    "editor:table-row-delete",
    "editor:table-col-before",
    "editor:table-col-after",
    "editor:table-col-left",
    "editor:table-col-right",
    "editor:table-col-copy",
    "editor:table-col-delete",
    "editor:table-col-align-left",
    "editor:table-col-align-center",
    "editor:table-col-align-right",
    "app:go-back",
    "app:go-forward",
    "app:open-settings",
    "app:open-help",
    "app:open-vault",
    "app:switch-vault",
    "app:open-another-vault",
    "app:open-sandbox-vault",
    "app:toggle-left-sidebar",
    "app:toggle-right-sidebar",
    "file-explorer:open",
    "file-explorer:reveal-active-file",
    "file-explorer:new-file",
    "file-explorer:new-file-in-current-tab",
    "file-explorer:new-file-in-new-pane",
    "file-explorer:new-folder",
    "file-explorer:move-file",
    "file-explorer:duplicate-file",
    "workspace:edit-file-title",
    "workspace:new-tab",
    "workspace:close",
    "workspace:close-others",
    "workspace:next-tab",
    "workspace:previous-tab",
    "workspace:goto-tab-1",
    "workspace:goto-tab-2",
    "workspace:goto-tab-3",
    "workspace:goto-tab-4",
    "workspace:goto-tab-5",
    "workspace:goto-tab-6",
    "workspace:goto-tab-7",
    "workspace:goto-tab-8",
    "workspace:goto-last-tab",
    "workspace:undo-close-pane",
    "workspace:split-vertical",
    "workspace:split-horizontal",
    "workspace:close-tab-group",
    "workspace:close-others-tab-group",
    "editor:focus-left",
    "editor:focus-right",
    "editor:focus-top",
    "editor:focus-bottom",
    "editor:open-link-in-new-split",
    "app:delete-file",
    "backlink:open",
    "backlink:open-backlinks",
    "outgoing-links:open",
    "outgoing-links:open-for-current",
    "outline:open",
    "outline:open-for-current",
    "tag-pane:open",
    "properties:open",
    "bookmarks:open",
    "bookmarks:bookmark-current-view",
    "bookmarks:bookmark-current-search",
    "bookmarks:unbookmark-current-view",
    "bookmarks:bookmark-current-section",
    "bookmarks:bookmark-current-heading",
    "bookmarks:bookmark-all-tabs",
];

/// Binds every runnable command's hotkeys in the Workspace context, and the
/// reading view's scrolling keys in its own; call once at startup with the
/// registry that has `hotkeys.json` applied.
pub fn bind_keys(cx: &mut App, registry: &CommandRegistry) {
    let ctx = Some("Workspace");
    let bindings: Vec<KeyBinding> = RUNNABLE
        .iter()
        .filter_map(|id| registry.get(id))
        .flat_map(|c| {
            c.hotkeys.iter().filter_map(|h| {
                let keys = keystroke(h)?;
                Some(KeyBinding::new(&keys, RunCommand { id: c.id.clone() }, ctx))
            })
        })
        .collect();
    cx.bind_keys(bindings);
    crate::reading::view::bind_keys(cx);
}

/// Everything derived from the vault folder: built once off the UI thread,
/// then updated in place by [`VaultState::apply`] as files change.
pub struct VaultState {
    pub root: PathBuf,
    vault: RwLock<Vault>,
    cache: RwLock<MetadataCache>,
    /// Note text as indexed, for search and mentions.
    texts: RwLock<HashMap<String, Arc<String>>>,
    /// The config files Settings writes, behind locks for the same reason
    /// `property_types` is: a control changes one while the vault is open.
    app: RwLock<AppConfig>,
    appearance: RwLock<AppearanceConfig>,
    core_plugins: RwLock<Option<CorePlugins>>,
    community_plugins: RwLock<Vec<String>>,
    switcher: RwLock<SwitcherConfig>,
    /// The Templates plugin's settings, for the two date commands and the
    /// template chooser.
    templates: RwLock<TemplatesConfig>,
    /// The Daily notes plugin's settings.
    daily_notes: RwLock<DailyNotesConfig>,
    /// Every installed plugin, classified into what Silica can run —
    /// native, in the compatibility host, or refused with a reason.
    pub plugins: silica_plugins::Registry,
    pub palette: CommandPaletteConfig,
    /// `types.json`: the type assigned to each property name.
    property_types: RwLock<PropertyTypes>,
    /// `bookmarks.json`, the Bookmarks plugin's items.
    bookmarks: RwLock<Bookmarks>,
    /// Config files that would not parse, so the app is running on
    /// defaults for them. Writing one is refused -- it would put those
    /// defaults on disk in place of what the user had -- and the window
    /// says so once, on the way in, rather than leaving the Settings tab
    /// looking merely wrong.
    pub unreadable_configs: Vec<String>,
    pub index_ms: f64,
}

/// One re-read note: its path and, for markdown, the new record and text.
type Reindexed = (String, Option<(CachedMetadata, Arc<String>)>);

/// What a batch of filesystem events changed.
#[derive(Debug, Clone, Default)]
pub struct Changes {
    pub files: Vec<FileChange>,
    /// Notes whose text was re-indexed.
    pub notes: Vec<String>,
    /// Notes that no longer exist.
    pub removed_notes: Vec<String>,
}

impl Changes {
    pub fn extend(&mut self, other: Changes) {
        self.files.extend(other.files);
        self.notes.extend(other.notes);
        self.removed_notes.extend(other.removed_notes);
    }

    fn structural(&self) -> bool {
        self.files
            .iter()
            .any(|c| !matches!(c, FileChange::Modified(_)))
    }
}

/// What a rename did: the files that moved and the notes whose links
/// were rewritten, on top of the index changes.
#[derive(Debug, Default)]
pub struct RenameOutcome {
    pub changes: Changes,
    pub renamed: Vec<(String, String)>,
    pub rewritten_files: usize,
    pub rewritten_links: usize,
    /// Notes whose links were left alone because the file on disk was not
    /// the text the rewrite was planned from.
    pub stale: Vec<String>,
}

impl VaultState {
    pub fn load(root: &PathBuf) -> Result<VaultState, String> {
        let started = Instant::now();
        let vault = Vault::open(root).map_err(|e| e.to_string())?;
        // Every config falls back to its defaults when it will not parse,
        // because one broken file may not stop a vault opening. Which ones
        // did is worth knowing, though: the window reports them, and a
        // write to any of them is refused rather than replacing it.
        let mut unreadable = Vec::new();
        // Only a parse failure counts. An unreadable file is a different
        // problem with a different message, and the write path lets it
        // through: there is nothing on disk to lose.
        let mut note = |name: &'static str, err: Option<&VaultError>| {
            if matches!(err, Some(VaultError::Json { .. })) {
                unreadable.push(name.to_string());
            }
        };
        let app = vault.app_config();
        note("app.json", app.as_ref().err());
        let app = app.unwrap_or_default();
        let appearance = vault.appearance_config();
        note("appearance.json", appearance.as_ref().err());
        let appearance = appearance.unwrap_or_default();
        let core_plugins = vault.core_plugins();
        note("core-plugins.json", core_plugins.as_ref().err());
        let core_plugins = core_plugins.unwrap_or_default();
        let community_plugins = vault.community_plugins();
        note("community-plugins.json", community_plugins.as_ref().err());
        let community_plugins = community_plugins.unwrap_or_default();
        let plugins = silica_plugins::Registry::scan(&vault);
        let palette = vault.command_palette_config();
        note("command-palette.json", palette.as_ref().err());
        let palette = palette.unwrap_or_default();
        let switcher = vault.switcher_config();
        note("switcher.json", switcher.as_ref().err());
        let switcher = switcher.unwrap_or_default();
        let templates = vault.templates_config();
        note("templates.json", templates.as_ref().err());
        let templates = templates.unwrap_or_default();
        let daily_notes = vault.daily_notes_config();
        note("daily-notes.json", daily_notes.as_ref().err());
        let daily_notes = daily_notes.unwrap_or_default();
        let property_types = vault.property_types();
        note("types.json", property_types.as_ref().err());
        let property_types = property_types.unwrap_or_default();
        let bookmarks = vault.bookmarks();
        note("bookmarks.json", bookmarks.as_ref().err());
        let bookmarks = bookmarks.unwrap_or_default();
        let notes: Vec<String> = vault.markdown_files().map(|f| f.path.clone()).collect();
        let loaded: Vec<(String, CachedMetadata, Arc<String>)> = notes
            .par_iter()
            .map(|path| {
                let text = std::fs::read_to_string(vault.absolute(path)).unwrap_or_default();
                let meta = index_markdown(&text);
                (path.clone(), meta, Arc::new(text))
            })
            .collect();
        let mut texts = HashMap::with_capacity(loaded.len());
        let metas: Vec<(String, CachedMetadata)> = loaded
            .into_iter()
            .map(|(path, meta, text)| {
                texts.insert(path.clone(), text);
                (path, meta)
            })
            .collect();
        let cache = MetadataCache::new(vault.link_resolver(), metas);
        Ok(VaultState {
            root: vault.root().to_path_buf(),
            vault: RwLock::new(vault),
            cache: RwLock::new(cache),
            texts: RwLock::new(texts),
            app: RwLock::new(app),
            appearance: RwLock::new(appearance),
            core_plugins: RwLock::new(core_plugins),
            community_plugins: RwLock::new(community_plugins),
            switcher: RwLock::new(switcher),
            templates: RwLock::new(templates),
            daily_notes: RwLock::new(daily_notes),
            plugins,
            palette,
            property_types: RwLock::new(property_types),
            bookmarks: RwLock::new(bookmarks),
            unreadable_configs: unreadable,
            index_ms: started.elapsed().as_secs_f64() * 1000.0,
        })
    }

    /// The vault's property type assignments.
    pub fn property_types(&self) -> RwLockReadGuard<'_, PropertyTypes> {
        self.property_types.read().expect("property types lock")
    }

    pub fn bookmarks(&self) -> RwLockReadGuard<'_, Bookmarks> {
        self.bookmarks.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Changes the bookmarks and writes `bookmarks.json` whole; the change
    /// is kept only once the file took it.
    pub fn update_bookmarks(&self, change: impl FnOnce(&mut Bookmarks)) -> Result<(), String> {
        let mut fresh = self.bookmarks().clone();
        change(&mut fresh);
        self.vault()
            .write_bookmarks(&fresh)
            .map_err(|e| e.to_string())?;
        *self.bookmarks.write().unwrap_or_else(|e| e.into_inner()) = fresh;
        Ok(())
    }

    /// Whether the Bookmarks core plugin is on: it is by default, and
    /// `core-plugins.json` can turn it off.
    pub fn bookmarks_enabled(&self) -> bool {
        self.core_plugin_enabled("bookmarks")
    }

    /// Whether a core plugin is on, by its id: what `core-plugins.json`
    /// says, or the plugin's own default when the file does not name it.
    pub fn core_plugin_enabled(&self, id: &str) -> bool {
        let Some(plugin) = CORE_PLUGINS.iter().find(|p| p.id == id) else {
            return true;
        };
        CorePlugins::is_enabled_or_default(self.core_plugins().as_ref(), plugin)
    }

    pub fn app(&self) -> RwLockReadGuard<'_, AppConfig> {
        self.app.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn appearance(&self) -> RwLockReadGuard<'_, AppearanceConfig> {
        self.appearance.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn core_plugins(&self) -> RwLockReadGuard<'_, Option<CorePlugins>> {
        self.core_plugins.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn community_plugins(&self) -> RwLockReadGuard<'_, Vec<String>> {
        self.community_plugins
            .read()
            .unwrap_or_else(|e| e.into_inner())
    }

    pub fn templates(&self) -> RwLockReadGuard<'_, TemplatesConfig> {
        self.templates.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn daily_notes(&self) -> RwLockReadGuard<'_, DailyNotesConfig> {
        self.daily_notes.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn switcher(&self) -> RwLockReadGuard<'_, SwitcherConfig> {
        self.switcher.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Writes a setting the Settings modal changed and keeps the config in
    /// hand in step. The file is written first: the vault is the source of
    /// truth, so a change that cannot reach disk does not count.
    pub fn apply_setting(&self, change: &SettingChange) -> Result<(), String> {
        match change {
            SettingChange::App(config) => {
                self.write_config("app.json", config)?;
                *self.app.write().unwrap_or_else(|e| e.into_inner()) = config.clone();
            }
            SettingChange::Appearance(config) => {
                self.write_config("appearance.json", config)?;
                *self.appearance.write().unwrap_or_else(|e| e.into_inner()) = config.clone();
            }
            SettingChange::Switcher(config) => {
                self.write_config("switcher.json", config)?;
                *self.switcher.write().unwrap_or_else(|e| e.into_inner()) = config.clone();
            }
            SettingChange::CorePlugins(config) => {
                self.write_config("core-plugins.json", config)?;
                *self.core_plugins.write().unwrap_or_else(|e| e.into_inner()) =
                    Some(config.clone());
            }
            SettingChange::CommunityPlugins(config) => {
                self.write_config("community-plugins.json", config)?;
                *self
                    .community_plugins
                    .write()
                    .unwrap_or_else(|e| e.into_inner()) = config.clone();
            }
            SettingChange::Hotkeys(overrides) => {
                // Whole, not merged: an entry taken out is a command back
                // on its defaults, and a merge would keep it.
                self.vault()
                    .replace_config("hotkeys.json", overrides)
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    fn write_config<T: serde::Serialize>(&self, name: &str, value: &T) -> Result<(), String> {
        self.vault()
            .write_config(name, value)
            .map_err(|e| e.to_string())
    }

    /// Records `key`'s type in `types.json`, as Obsidian's type menu does.
    /// Returns the table as it now stands so the views can redraw from it.
    pub fn assign_property_type(
        &self,
        key: &str,
        ty: silica_formats::properties::PropertyType,
    ) -> Result<PropertyTypes, String> {
        let mut types = self.property_types.write().expect("property types lock");
        types.types.insert(key.to_string(), ty.as_str().to_string());
        self.vault()
            .write_config("types.json", &*types)
            .map_err(|e| e.to_string())?;
        Ok(types.clone())
    }

    pub fn vault(&self) -> RwLockReadGuard<'_, Vault> {
        self.vault.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn cache(&self) -> RwLockReadGuard<'_, MetadataCache> {
        self.cache.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn texts(&self) -> RwLockReadGuard<'_, HashMap<String, Arc<String>>> {
        self.texts.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn build_options(&self) -> reading::BuildOptions {
        reading::BuildOptions {
            strict_line_breaks: self.app().strict_line_breaks.unwrap_or(false),
        }
    }

    /// Applies filesystem events for `paths`: refreshes the file list,
    /// re-indexes changed notes, and re-resolves links when files came or went.
    pub fn apply(&self, paths: &[PathBuf]) -> Changes {
        let mut files = Vec::new();
        {
            let mut vault = self.vault.write().unwrap_or_else(|e| e.into_inner());
            for path in paths {
                files.extend(vault.refresh_path(path));
            }
        }
        self.absorb(files)
    }

    /// Re-reads and re-indexes the notes among `files` (already applied to
    /// the file list) and re-resolves links when files came or went.
    pub(crate) fn absorb(&self, files: Vec<FileChange>) -> Changes {
        let mut changes = Changes::default();
        // Keep one change per path, the last one seen.
        let mut last: HashMap<String, FileChange> = HashMap::new();
        for c in files {
            last.insert(c.path().to_string(), c);
        }
        let mut files: Vec<FileChange> = last.into_values().collect();
        files.sort_by(|a, b| a.path().cmp(b.path()));
        if files.is_empty() {
            return changes;
        }
        let reread: Vec<Reindexed> = files
            .par_iter()
            .map(|c| match c {
                FileChange::Added(p) | FileChange::Modified(p)
                    if FileKind::from_path(p) == FileKind::Markdown =>
                {
                    let text = std::fs::read_to_string(self.root.join(p)).unwrap_or_default();
                    let meta = index_markdown(&text);
                    (p.clone(), Some((meta, Arc::new(text))))
                }
                other => (other.path().to_string(), None),
            })
            .collect();
        {
            let mut cache = self.cache.write().unwrap_or_else(|e| e.into_inner());
            let mut texts = self.texts.write().unwrap_or_else(|e| e.into_inner());
            for (c, (path, indexed)) in files.iter().zip(reread) {
                match (c, indexed) {
                    (FileChange::Removed(_), _) => {
                        if texts.remove(&path).is_some() {
                            changes.removed_notes.push(path.clone());
                        }
                        cache.forget_path(&path);
                    }
                    (_, Some((meta, text))) => {
                        cache.insert_file(path.clone(), meta);
                        texts.insert(path.clone(), text);
                        changes.notes.push(path);
                    }
                    (_, None) => cache.add_path(&path),
                }
            }
            if files.iter().any(|c| !matches!(c, FileChange::Modified(_))) {
                cache.reresolve();
            }
        }
        changes.files = files;
        changes
    }

    pub fn link_format(&self) -> LinkFormat {
        LinkFormat::from_config(self.app().new_link_format.as_deref())
    }

    pub fn trash_option(&self) -> TrashOption {
        TrashOption::from_config(self.app().trash_option.as_deref())
    }

    /// `alwaysUpdateLinks`: on by default; off means ask before rewriting.
    pub fn update_links_automatically(&self) -> bool {
        self.app().always_update_links.unwrap_or(true)
    }

    /// `promptDelete`: on by default.
    pub fn confirm_delete(&self) -> bool {
        self.app().prompt_delete.unwrap_or(true)
    }

    /// Where a note created for the link text `name` goes, given the open
    /// note (`newFileLocation`, `newFileFolderPath`).
    pub fn new_note_path(&self, name: &str, current: Option<&str>) -> String {
        let vault = self.vault();
        let folder = vault.new_note_folder(
            self.app().new_file_location.as_deref(),
            self.app().new_file_folder_path.as_deref(),
            current,
        );
        vault.new_note_path(name, &folder)
    }

    /// Creates a note (and its folders) and indexes it.
    pub fn create_note(&self, path: &str, text: &str) -> Result<Changes, String> {
        let files = self
            .vault
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .create_note(path, text)
            .map_err(|e| e.to_string())?;
        Ok(self.absorb(files))
    }

    /// Saves an attachment file (and creates its folders) and indexes it.
    pub fn save_attachment(&self, path: &str, bytes: &[u8]) -> Result<Changes, String> {
        let files = self
            .vault
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .save_attachment(path, bytes)
            .map_err(|e| e.to_string())?;
        Ok(self.absorb(files))
    }

    pub fn create_folder(&self, path: &str) -> Result<(), String> {
        self.vault
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .create_folder(path)
            .map_err(|e| e.user_message())
    }

    /// Copies a file next to itself as `Name 1.ext` and indexes the copy.
    pub fn duplicate(&self, path: &str) -> Result<(String, Changes), String> {
        let (target, from, to) = {
            let vault = self.vault();
            let (folder, name) = match path.rsplit_once('/') {
                Some((d, n)) => (d, n),
                None => ("", path),
            };
            let (stem, ext) = match name.rsplit_once('.') {
                Some((s, e)) if !s.is_empty() => (s, e),
                _ => (name, ""),
            };
            let target = vault.available_path(folder, stem, ext);
            let to = vault.write_path(&target).map_err(|e| e.to_string())?;
            (target.clone(), vault.absolute(path), to)
        };
        std::fs::copy(&from, &to).map_err(|e| format!("Could not copy {path}: {e}"))?;
        let files = self
            .vault
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .refresh_path(&to);
        Ok((target, self.absorb(files)))
    }

    /// The rewrites a rename would need, without doing anything:
    /// `(files, links)`.
    pub fn rename_preview(&self, from: &str, to: &str) -> (usize, usize) {
        let plan = self.vault().rename_plan(from, to);
        let texts = self.texts();
        let rewrites = self
            .cache()
            .plan_rename_rewrites(&plan, self.link_format(), &|p| {
                texts.get(p).map(|t| t.to_string())
            });
        (
            rewrites.len(),
            rewrites.iter().map(|r| r.links).sum::<usize>(),
        )
    }

    /// Renames or moves a file or folder, rewrites the links to it in every
    /// note (when `update_links`), and brings the index up to date.
    /// Writes a block id into `path`, which is a note the user is not
    /// editing. The bytes on disk have to be the bytes the insertion was
    /// planned against: the vault is the source of truth, and a note that
    /// moved underneath is left alone rather than overwritten.
    ///
    /// The file is written and the watcher re-indexes it, which is the same
    /// path a save takes.
    pub fn write_block_id(
        &self,
        path: &str,
        before: &str,
        insertion: &silica_suggest::blocks::IdInsertion,
    ) -> Result<(), String> {
        let absolute = self.vault().write_path(path).map_err(|e| e.to_string())?;
        let on_disk = std::fs::read_to_string(&absolute)
            .map_err(|e| format!("Could not read {path}: {e}"))?;
        if on_disk != before {
            return Err(format!("{path} changed on disk; no id was written"));
        }
        silica_vault::write_no_follow(&absolute, insertion.applied(&on_disk).as_bytes())
            .map_err(|e| format!("Could not write {path}: {e}"))
    }

    pub fn rename(
        &self,
        from: &str,
        to: &str,
        update_links: bool,
    ) -> Result<RenameOutcome, String> {
        let moved = self
            .vault
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .rename(from, to)
            // Both of these end up in a modal the user typed into, so they
            // get the sentence rather than the path and the errno.
            .map_err(|e| e.user_message())?;
        let plan = if update_links {
            let texts = self.texts();
            self.cache()
                .plan_rename_rewrites(&moved, self.link_format(), &|p| {
                    texts.get(p).map(|t| t.to_string())
                })
        } else {
            Vec::new()
        };
        // Each rewrite was planned from the text the index holds, so the
        // file has to still be that text: another app may have saved it
        // since, and writing the plan would throw that away. A note that
        // moved underneath keeps its own bytes and its links are left for
        // the user, which loses nothing either way.
        let mut stale: Vec<String> = Vec::new();
        let texts = self.texts();
        for rw in &plan {
            let absolute = self
                .vault()
                .write_path(&rw.new_path)
                .map_err(|e| e.to_string())?;
            let planned_from = texts.get(&rw.old_path).map(|t| t.to_string());
            match (std::fs::read_to_string(&absolute), planned_from) {
                (Ok(on_disk), Some(was)) if on_disk != was => {
                    stale.push(rw.new_path.clone());
                    continue;
                }
                _ => {}
            }
            silica_vault::write_no_follow(&absolute, rw.text.as_bytes())
                .map_err(|e| format!("Could not update links in {}: {e}", rw.new_path))?;
        }
        drop(texts);
        let moved_from: HashMap<&str, &str> = moved
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let mut changes = Changes::default();
        let mut cache = self.cache.write().unwrap_or_else(|e| e.into_inner());
        let mut texts = self.texts.write().unwrap_or_else(|e| e.into_inner());
        for (old, new) in &moved {
            let old_text = texts.remove(old);
            cache.forget_path(old);
            changes.files.push(FileChange::Removed(old.clone()));
            if old_text.is_some() {
                changes.removed_notes.push(old.clone());
            }
            if FileKind::from_path(new) == FileKind::Markdown {
                let text = plan
                    .iter()
                    .find(|r| r.old_path == *old)
                    .map(|r| Arc::new(r.text.clone()))
                    .or(old_text)
                    .unwrap_or_else(|| {
                        Arc::new(std::fs::read_to_string(self.root.join(new)).unwrap_or_default())
                    });
                cache.insert_file(new.clone(), index_markdown(&text));
                texts.insert(new.clone(), text);
                changes.notes.push(new.clone());
            } else {
                cache.add_path(new);
            }
            changes.files.push(FileChange::Added(new.clone()));
        }
        for rw in plan
            .iter()
            .filter(|r| !moved_from.contains_key(r.old_path.as_str()))
        {
            let text = Arc::new(rw.text.clone());
            cache.insert_file(rw.new_path.clone(), index_markdown(&text));
            texts.insert(rw.new_path.clone(), text);
            changes
                .files
                .push(FileChange::Modified(rw.new_path.clone()));
            changes.notes.push(rw.new_path.clone());
        }
        cache.reresolve();
        Ok(RenameOutcome {
            changes,
            renamed: moved,
            rewritten_files: plan.len() - stale.len(),
            rewritten_links: plan
                .iter()
                .filter(|r| !stale.contains(&r.new_path))
                .map(|r| r.links)
                .sum(),
            stale,
        })
    }

    /// Deletes a file or folder per `trashOption` and drops it from the index.
    pub fn trash(&self, path: &str) -> Result<Changes, String> {
        let removed = self
            .vault
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .trash(path, self.trash_option())
            .map_err(|e| e.to_string())?;
        let mut changes = Changes::default();
        let mut cache = self.cache.write().unwrap_or_else(|e| e.into_inner());
        let mut texts = self.texts.write().unwrap_or_else(|e| e.into_inner());
        for p in removed {
            if texts.remove(&p).is_some() {
                changes.removed_notes.push(p.clone());
            }
            cache.forget_path(&p);
            changes.files.push(FileChange::Removed(p));
        }
        cache.reresolve();
        Ok(changes)
    }

    /// Note names and aliases, for unlinked mentions.
    fn names(&self) -> Vec<(String, String)> {
        let cache = self.cache();
        let mut out = Vec::new();
        for (path, meta) in cache.files() {
            let base = path
                .rsplit('/')
                .next()
                .unwrap_or(path)
                .trim_end_matches(".md");
            out.push((base.to_string(), path.to_string()));
            for alias in frontmatter_aliases(meta) {
                out.push((alias, path.to_string()));
            }
        }
        out
    }
}

impl reading::EmbedSource for VaultState {
    fn resolve(&self, linktext: &str, source_path: &str) -> Option<String> {
        self.cache()
            .resolve(linktext, source_path)
            .map(str::to_string)
    }

    fn read(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(self.vault().absolute(path)).ok()
    }

    fn absolute(&self, path: &str) -> PathBuf {
        self.vault().absolute(path)
    }

    /// An embedded `![[x.base]]`: the file resolved against this vault, on
    /// its first view. The whole index and cache are behind it, which is why
    /// the reading model asks for this rather than for the bytes.
    fn base(&self, path: &str) -> Option<crate::bases::EmbeddedBase> {
        crate::bases::resolve_embed(self, path)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Reading,
    /// Editing with markers hidden and constructs rendered in place.
    LivePreview,
    /// Editing the raw text.
    Source,
    Graph,
}

impl ViewMode {
    /// Both editing modes show the editor.
    pub fn is_editing(self) -> bool {
        matches!(self, ViewMode::LivePreview | ViewMode::Source)
    }

    /// The name a mode goes by outside the app: the control socket's
    /// `set_mode` and the `mode` in its state dump.
    pub fn as_str(self) -> &'static str {
        match self {
            ViewMode::Reading => "reading",
            ViewMode::LivePreview => "live",
            ViewMode::Source => "source",
            ViewMode::Graph => "graph",
        }
    }

    /// The mode one of those names stands for; `None` for anything else,
    /// so a bad request is answered rather than guessed at.
    pub fn parse(name: &str) -> Option<ViewMode> {
        match name {
            "reading" => Some(ViewMode::Reading),
            "live" => Some(ViewMode::LivePreview),
            "source" => Some(ViewMode::Source),
            "graph" => Some(ViewMode::Graph),
            _ => None,
        }
    }
}

/// What the main area paints. The mode asks for a view; the open file
/// decides whether that view has anything to show, and `Empty` is the
/// placeholder for when it does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MainView {
    Reading,
    Editor,
    Graph,
    Canvas,
    Bases,
    Pdf,
    Empty,
}

impl MainView {
    /// The name the socket's `focus` and `state` use for this view.
    fn as_str(self) -> &'static str {
        match self {
            MainView::Reading => "reading",
            MainView::Editor => "editor",
            MainView::Graph => "graph",
            MainView::Canvas => "canvas",
            MainView::Bases => "bases",
            MainView::Pdf => "pdf",
            MainView::Empty => "nothing",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeftTab {
    Files,
    Search,
    Bookmarks,
}

/// A bookmark being made: what it will point at, before the title prompt
/// has been answered.
#[derive(Debug, Clone, PartialEq, Eq)]
enum BookmarkDraft {
    File {
        path: String,
        subpath: Option<String>,
    },
    Folder(String),
    Search(String),
    Graph,
    /// "Bookmark all tabs": a group of these notes.
    Tabs(Vec<String>),
}

impl BookmarkDraft {
    /// The text the prompt's Title field shows as its placeholder, which is
    /// also what the pane shows when no title is typed.
    fn placeholder(&self) -> String {
        match self {
            BookmarkDraft::File { path, subpath } => {
                Bookmark::file(path, subpath.as_deref(), None, 0).display_title()
            }
            BookmarkDraft::Folder(path) => path.clone(),
            BookmarkDraft::Search(query) => query.clone(),
            BookmarkDraft::Graph => "Untitled Graph".to_string(),
            BookmarkDraft::Tabs(_) => templates::format("MMMM D, YYYY h:mm A", &now()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RightTab {
    Backlinks,
    Outgoing,
    Tags,
    /// All properties: every key in the vault with its type and count.
    Properties,
    Outline,
}

/// The Outline pane's headings for one note's metadata, whether that
/// metadata came from the index or from the buffer being typed into.
fn outline_headings(meta: &CachedMetadata) -> Vec<panes::Heading> {
    meta.headings
        .iter()
        .map(|h| panes::Heading {
            text: Workspace::heading_text(&h.heading),
            level: h.level,
            line: h.position.start.line as usize,
            offset: h.position.start.offset,
        })
        .collect()
}

/// What a tree row's fold triangle does when clicked.
type FoldAction = Box<dyn Fn(&mut Workspace, &mut Context<Workspace>) + 'static>;

/// The Tags leaf's state: the four keys Obsidian keeps for it in
/// `workspace.json`, plus the folds, which it does not save.
#[derive(Debug, Clone)]
struct TagPaneState {
    /// `sortOrder`.
    sort: PaneSort,
    /// `useHierarchy`: `a/b` drawn as `b` under `a`, not as one flat row.
    use_hierarchy: bool,
    /// `showSearch`; the query itself lives in the filter field.
    show_search: bool,
    /// Folded tags by lowercased path.
    collapsed: HashSet<String>,
    sort_menu_open: bool,
}

impl Default for TagPaneState {
    fn default() -> TagPaneState {
        TagPaneState {
            sort: PaneSort::Frequency,
            use_hierarchy: true,
            show_search: false,
            collapsed: HashSet::new(),
            sort_menu_open: false,
        }
    }
}

/// The All properties leaf's state: `sortOrder` and `showSearch` (the
/// `searchQuery` is the filter field's text).
#[derive(Debug, Clone, Default)]
struct PropertyPaneState {
    sort: PaneSort,
    show_search: bool,
    sort_menu_open: bool,
}

/// The Outline leaf's state: `followCursor` and `showSearch` from
/// `workspace.json` (`file` is the open note), plus what it does not save:
/// the folds, which go when the note changes, and the row marked as
/// holding the cursor.
#[derive(Default)]
struct OutlinePaneState {
    /// `followCursor`: the pane scrolls the marked row into view.
    follow_cursor: bool,
    show_search: bool,
    /// Folded headings by index into the outline.
    collapsed: HashSet<usize>,
    /// The note the folds belong to.
    file: Option<String>,
    /// The heading whose section holds the cursor, as last computed.
    active: Option<usize>,
    /// The mark moved (or the note changed): the next draw scrolls to it
    /// when `follow_cursor` is on.
    scroll_pending: bool,
    scroll: ScrollHandle,
}

/// Mentions computed off the UI thread for one open note.
struct MentionSections {
    path: String,
    incoming: Section,
    outgoing: Section,
}

/// The Backlinks leaf's own state: the keys Obsidian keeps for it in
/// `workspace.json`, plus which groups were toggled by hand.
#[derive(Debug, Clone)]
struct BacklinkState {
    /// `collapseAll`: every group starts collapsed.
    collapse_all: bool,
    /// `extraContext`: a match shows the lines either side of its own.
    extra_context: bool,
    /// `sortOrder`, the Files view's six orders.
    sort: SortOrder,
    /// `showSearch`: the filter field is on screen.
    show_search: bool,
    /// `backlinkCollapsed`: the Linked mentions section is folded.
    linked_collapsed: bool,
    /// `unlinkedCollapsed`.
    unlinked_collapsed: bool,
    /// Groups toggled away from what `collapse_all` says, by
    /// [`Section::group_key`].
    toggled: HashSet<String>,
    sort_menu_open: bool,
}

impl Default for BacklinkState {
    fn default() -> BacklinkState {
        BacklinkState {
            collapse_all: false,
            extra_context: false,
            sort: SortOrder::Alphabetical,
            show_search: false,
            linked_collapsed: false,
            // Obsidian's fresh leaf starts with the section folded.
            unlinked_collapsed: true,
            toggled: HashSet::new(),
            sort_menu_open: false,
        }
    }
}

/// The Outgoing links leaf's state: `linksCollapsed` and
/// `unlinkedCollapsed`, plus the groups folded by hand.
#[derive(Debug, Clone)]
struct OutgoingState {
    links_collapsed: bool,
    unlinked_collapsed: bool,
    toggled: HashSet<String>,
}

impl Default for OutgoingState {
    fn default() -> OutgoingState {
        OutgoingState {
            links_collapsed: false,
            // Obsidian's fresh leaf starts with the section folded.
            unlinked_collapsed: true,
            toggled: HashSet::new(),
        }
    }
}

/// How long a transient message stays on screen. Obsidian reports what a
/// command did with a Notice that fades on its own after a few seconds;
/// the exact duration there has not been probed.
const NOTICE_LIFETIME: Duration = Duration::from_secs(5);

/// A message on screen, with the id its expiry timer names it by.
struct Notice {
    id: u64,
    text: SharedString,
}

/// The transient messages, oldest first. The status bar is for what stays
/// true — the open file, the index summary — so everything a command has
/// to say goes here instead and leaves again on its own.
#[derive(Default)]
struct Notices {
    live: Vec<Notice>,
    last_id: u64,
}

impl Notices {
    /// Shows `text` and returns the id to expire it by. A message already
    /// on screen is renewed rather than stacked: the autosave retries on a
    /// timer, so the same refusal arrives over and over.
    fn push(&mut self, text: SharedString) -> u64 {
        self.live.retain(|n| n.text != text);
        self.last_id += 1;
        self.live.push(Notice {
            id: self.last_id,
            text,
        });
        self.last_id
    }

    /// Drops the message with this id, if it is still the one on screen.
    fn expire(&mut self, id: u64) {
        self.live.retain(|n| n.id != id);
    }

    fn clear(&mut self) {
        self.live.clear();
    }

    /// The newest message, which is what the last command said.
    fn latest(&self) -> Option<&SharedString> {
        self.live.last().map(|n| &n.text)
    }

    fn texts(&self) -> Vec<String> {
        self.live.iter().map(|n| n.text.to_string()).collect()
    }
}

/// Which overlay is up, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Modal {
    None,
    Switcher,
    Palette,
    Settings,
    Prompt,
    /// The template chooser.
    Chooser,
    /// The Search pane's "Copy search results".
    CopyResults,
}

/// The file operation the open prompt will finish.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingOp {
    Rename(String),
    Move(String),
    Delete(String),
    NewFolder {
        parent: String,
    },
    /// `alwaysUpdateLinks` is off: asking whether to rewrite links.
    UpdateLinks {
        from: String,
        to: String,
    },
    /// "Add bookmark": the title for this draft.
    AddBookmark(BookmarkDraft),
    /// "Edit bookmark" on a note that already has one: Save keeps it,
    /// Remove takes it out.
    EditBookmark(String),
}

/// One tab of the main area: what it shows and where it has been. The
/// active tab's fields live on the workspace itself — `current`, `mode`,
/// `history`, `history_ix` — so every path that opens a note keeps working
/// unchanged; `tabs[active_tab]` is a snapshot of them, refreshed by
/// [`Workspace::stash_active_tab`] before anything reads it.
/// One place back and forward can land: the note, and the state the view
/// was left in. Obsidian's history entries carry the same -- the mode and
/// the scroll -- which is why going back to a note read in Reading brings
/// it back in Reading, where it was.
#[derive(Debug, Clone, PartialEq)]
struct HistoryEntry {
    path: String,
    /// The mode the note was in. Graph is never recorded: it is a toggle
    /// over the note rather than a place a note can be.
    mode: ViewMode,
    /// Where in the note the reader was, as a byte offset: the caret in an
    /// editing mode, the top block in Reading.
    offset: usize,
}

impl HistoryEntry {
    /// A note with nothing known about it yet, for the entry an open
    /// pushes before the view has settled.
    fn new(path: String, mode: ViewMode) -> HistoryEntry {
        HistoryEntry {
            path,
            mode,
            offset: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Tab {
    path: Option<String>,
    mode: ViewMode,
    history: Vec<HistoryEntry>,
    history_ix: usize,
    /// The `workspace.json` leaf this tab was restored from, so a layout
    /// nothing changed is written back with the keys Obsidian gave it.
    leaf: Option<Value>,
    /// A canvas tab's `viewState` (`x`, `y`, `zoom`): read from the leaf,
    /// kept current by the view's [`CanvasEvent::Viewport`] while the tab
    /// is on screen, and written back into the leaf. It lives here rather
    /// than being read from the view when the layout is written, because a
    /// tab that is not live has no view holding it. A PDF tab has none: its
    /// leaf is the file alone (observed), and the PDF view remembers its
    /// own place for the session.
    view_state: Option<Value>,
}

impl Tab {
    fn empty(mode: ViewMode) -> Tab {
        Tab {
            path: None,
            mode,
            history: Vec::new(),
            history_ix: 0,
            leaf: None,
            view_state: None,
        }
    }

    /// The tab's title as the strip shows it: the note's name, or Obsidian's
    /// "New tab" for one with nothing in it.
    fn title(&self) -> SharedString {
        leaf_title(self.path.as_deref(), self.mode)
    }
}

/// What the chooser is open for: a template to insert, a vault file for a
/// canvas card, or a group to jump to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ChooserPurpose {
    #[default]
    Template,
    CanvasFile,
    CanvasGroup,
    /// `bases:change-view`: the showing base's views, by index.
    BasesView,
}

/// How Obsidian names a split's axis: a `vertical` split lays its children
/// out side by side, a `horizontal` one stacks them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SplitDirection {
    Vertical,
    Horizontal,
}

impl SplitDirection {
    fn as_str(self) -> &'static str {
        match self {
            SplitDirection::Vertical => "vertical",
            SplitDirection::Horizontal => "horizontal",
        }
    }

    fn parse(name: Option<&str>) -> SplitDirection {
        match name {
            Some("horizontal") => SplitDirection::Horizontal,
            _ => SplitDirection::Vertical,
        }
    }
}

/// The main area's shape: a tree of splits over the tab groups, the way
/// `workspace.json`'s `main` records it. The root is always a split, as
/// Obsidian's is, even around a single group.
#[derive(Debug, Clone, PartialEq)]
enum Layout {
    Group(u64),
    Split {
        direction: SplitDirection,
        children: Vec<Layout>,
        /// The split's own keys from `workspace.json` (`id`, `dimension`,
        /// anything unknown), kept so the node is written back as read.
        node: Map<String, Value>,
    },
}

impl Layout {
    fn split(direction: SplitDirection, children: Vec<Layout>) -> Layout {
        let mut node = Map::new();
        node.insert("id".into(), json!(leaf_id()));
        node.insert("type".into(), json!("split"));
        Layout::Split {
            direction,
            children,
            node,
        }
    }

    /// Every group, depth first, in the order Obsidian lays them out.
    fn groups(&self, out: &mut Vec<u64>) {
        match self {
            Layout::Group(id) => out.push(*id),
            Layout::Split { children, .. } => children.iter().for_each(|c| c.groups(out)),
        }
    }

    /// The group in the main area's top right corner: the last child of
    /// every side-by-side split and the first of every stacked one. Its
    /// strip is the one Obsidian's `mod-top-right-space` pads for the
    /// collapsed right sidebar's toggle and the window controls.
    fn top_right(&self) -> Option<u64> {
        match self {
            Layout::Group(id) => Some(*id),
            Layout::Split {
                direction,
                children,
                ..
            } => match direction {
                SplitDirection::Vertical => children.last()?.top_right(),
                SplitDirection::Horizontal => children.first()?.top_right(),
            },
        }
    }

    fn contains(&self, id: u64) -> bool {
        match self {
            Layout::Group(g) => *g == id,
            Layout::Split { children, .. } => children.iter().any(|c| c.contains(id)),
        }
    }

    /// The child indexes from the root down to group `id`.
    fn path_to(&self, id: u64) -> Option<Vec<usize>> {
        match self {
            Layout::Group(g) => (*g == id).then(Vec::new),
            Layout::Split { children, .. } => children.iter().enumerate().find_map(|(ix, c)| {
                let mut path = c.path_to(id)?;
                path.insert(0, ix);
                Some(path)
            }),
        }
    }

    fn node_at(&self, path: &[usize]) -> Option<&Layout> {
        match path.split_first() {
            None => Some(self),
            Some((ix, rest)) => match self {
                Layout::Split { children, .. } => children.get(*ix)?.node_at(rest),
                Layout::Group(_) => None,
            },
        }
    }

    fn node_at_mut(&mut self, path: &[usize]) -> Option<&mut Layout> {
        match path.split_first() {
            None => Some(self),
            Some((ix, rest)) => match self {
                Layout::Split { children, .. } => children.get_mut(*ix)?.node_at_mut(rest),
                Layout::Group(_) => None,
            },
        }
    }

    /// Puts `new` beside group `id` along `direction`, the way "Split right"
    /// and "Split down" do: a sibling when the enclosing split already runs
    /// that way, otherwise the group is wrapped in a new split holding the
    /// two of them. `after` says which side: right or below when set, left
    /// or above when not (a tab dropped on a group's near edge). Says which
    /// it was (`Some(true)` for wrapped), so the caller can hand the group's
    /// `dimension` on.
    fn insert_beside(
        &mut self,
        id: u64,
        new: u64,
        direction: SplitDirection,
        after: bool,
    ) -> Option<bool> {
        let Layout::Split {
            direction: own,
            children,
            ..
        } = self
        else {
            return None;
        };
        let ix = children.iter().position(|c| c.contains(id))?;
        if matches!(children[ix], Layout::Group(g) if g == id) {
            if *own == direction {
                children.insert(if after { ix + 1 } else { ix }, Layout::Group(new));
                return Some(false);
            }
            let pair = if after {
                vec![Layout::Group(id), Layout::Group(new)]
            } else {
                vec![Layout::Group(new), Layout::Group(id)]
            };
            children[ix] = Layout::split(direction, pair);
            return Some(true);
        }
        children[ix].insert_beside(id, new, direction, after)
    }

    /// Takes group `id` out. A split left with one child keeps it (Obsidian
    /// does not collapse the nesting), one left with none goes with it.
    fn remove(&mut self, id: u64) -> bool {
        let Layout::Split { children, .. } = self else {
            return false;
        };
        let Some(ix) = children.iter().position(|c| c.contains(id)) else {
            return false;
        };
        if matches!(children[ix], Layout::Group(g) if g == id) {
            children.remove(ix);
            return true;
        }
        let removed = children[ix].remove(id);
        if removed && matches!(&children[ix], Layout::Split { children, .. } if children.is_empty())
        {
            children.remove(ix);
        }
        removed
    }

    /// The groups to step to from `id` along `direction`: the next (or
    /// previous) sibling of the nearest enclosing split that runs that way,
    /// as a set the caller picks the most recently used member of.
    fn neighbors(&self, id: u64, direction: SplitDirection, forward: bool) -> Vec<u64> {
        let Some(path) = self.path_to(id) else {
            return Vec::new();
        };
        for depth in (0..path.len()).rev() {
            let Some(Layout::Split {
                direction: own,
                children,
                ..
            }) = self.node_at(&path[..depth])
            else {
                continue;
            };
            if *own != direction {
                continue;
            }
            let ix = path[depth];
            let sibling = if forward {
                children.get(ix + 1)
            } else {
                ix.checked_sub(1).and_then(|i| children.get(i))
            };
            if let Some(sibling) = sibling {
                let mut out = Vec::new();
                sibling.groups(&mut out);
                return out;
            }
        }
        Vec::new()
    }
}

/// One tab group: a strip of tabs with its own editor and reading view, so
/// two groups can show two notes at once. The active group's tabs and the
/// note on screen live in the workspace's own fields; a group that is not
/// active keeps its own here, put there by [`Workspace::stash_active_group`]
/// when another group took over.
struct TabGroup {
    id: u64,
    tabs: Vec<Tab>,
    active_tab: usize,
    mode_before_graph: ViewMode,
    detached: Option<SharedString>,
    words: usize,
    chars: usize,
    editor: Entity<Editor>,
    reading: Entity<ReadingView>,
    canvas: Entity<CanvasView>,
    bases: Entity<BasesView>,
    pdf: Entity<PdfView>,
    /// The `tabs` node this group was restored from, less its children:
    /// `id`, a `dimension` from a dragged divider, and anything unknown.
    node: Map<String, Value>,
}

impl TabGroup {
    fn new(
        id: u64,
        editor: Entity<Editor>,
        reading: Entity<ReadingView>,
        canvas: Entity<CanvasView>,
        bases: Entity<BasesView>,
        pdf: Entity<PdfView>,
        mode: ViewMode,
    ) -> TabGroup {
        let mut node = Map::new();
        node.insert("id".into(), json!(leaf_id()));
        node.insert("type".into(), json!("tabs"));
        TabGroup {
            id,
            tabs: vec![Tab::empty(mode)],
            active_tab: 0,
            mode_before_graph: ViewMode::Reading,
            detached: None,
            words: 0,
            chars: 0,
            editor,
            reading,
            canvas,
            bases,
            pdf,
            node,
        }
    }

    /// The group's share of its split, Obsidian's `dimension` (a percentage).
    fn dimension(&self) -> Option<f64> {
        self.node.get("dimension").and_then(Value::as_f64)
    }

    fn set_dimension(&mut self, dimension: Option<f64>) {
        set_dimension(&mut self.node, dimension);
    }
}

/// Writes or drops a node's `dimension`, in the slot Obsidian gives it:
/// after `type`, before `children`.
fn set_dimension(node: &mut Map<String, Value>, dimension: Option<f64>) {
    match dimension {
        Some(d) => {
            if node.contains_key("dimension") {
                node["dimension"] = json!(d);
            } else {
                let at = node
                    .keys()
                    .position(|k| k == "type")
                    .map(|i| i + 1)
                    .unwrap_or(node.len());
                node.shift_insert(at, "dimension".into(), json!(d));
            }
        }
        None => {
            node.shift_remove("dimension");
        }
    }
}

/// What one group's rendering reads, whichever group it is: the live
/// fields for the active group, the stashed ones for the others.
struct GroupView {
    id: u64,
    active: bool,
    tabs: Vec<Tab>,
    active_tab: usize,
    current: Option<String>,
    mode: ViewMode,
    history_len: usize,
    history_ix: usize,
    detached: Option<SharedString>,
    editor: Entity<Editor>,
    reading: Entity<ReadingView>,
    canvas: Entity<CanvasView>,
    bases: Entity<BasesView>,
    pdf: Entity<PdfView>,
}

/// The view-header title being renamed in place.
///
/// Obsidian's `.view-header-title` is editable in its resting state, so a
/// single click is all it takes to start typing in it: the title changes
/// under the pointer while the file keeps its old name, and only Enter
/// renames the file (see
/// [[Specification#The view-header title is an inline rename field]]).
/// The text is the name without its extension, as the rename prompt's box
/// holds it, so both commit through the same path and the same validator.
struct TitleEdit {
    /// The group whose header holds the field.
    group: u64,
    /// The file the field opened on. The edit is dropped if that leaf moves
    /// on, so a commit can only ever rename the file the title showed.
    path: String,
    field: TitleField,
    /// The composition the platform's input method has in the field, if
    /// any: the bytes it will replace with the committed text.
    marked: Option<Range<usize>>,
}

/// Where a group's header title was last painted and how its text was
/// shaped: what a click is measured against to find the offset it landed
/// on, and what the input method is told when it asks where the text is.
#[derive(Clone)]
struct TitleLayout {
    bounds: Bounds<Pixels>,
    line: ShapedLine,
    line_height: Pixels,
    scroll_x: Pixels,
}

/// What a header title works out before it paints.
struct TitlePrepaint {
    line: ShapedLine,
    cursor_x: Pixels,
    /// The selection's two edges in the shaped line, when there is one.
    selection_x: Option<Range<Pixels>>,
    scroll_x: Pixels,
    line_height: Pixels,
    /// An edit is open on this group, so the platform's input method is
    /// pointed at the field.
    editing: bool,
    /// The field has the keyboard, so it carries a caret.
    caret: bool,
}

/// The payload of a divider drag: which split, and the boundary between
/// which two of its children.
#[derive(Debug, Clone)]
struct DividerDrag {
    path: Vec<usize>,
    before: usize,
}

/// The payload of a tab drag: which group's strip the tab was picked up
/// from and which tab it is. The tab itself stays in its group until the
/// drop; a drag that ends nowhere changes nothing.
#[derive(Debug, Clone)]
struct TabDrag {
    group: u64,
    tab: usize,
    title: SharedString,
}

/// Where a tab dragged over a group's content would land: in the group's
/// strip, or in a new group on one of its sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DropZone {
    Center,
    Left,
    Right,
    Top,
    Bottom,
}

impl DropZone {
    /// The zone `position` falls in inside `bounds`: the outer band on each
    /// side is a split, the middle is the strip. The band is a share of the
    /// shorter side so a tall narrow group still has a middle; which edge
    /// wins in a corner is the nearer one.
    fn at(bounds: gpui::Bounds<gpui::Pixels>, position: gpui::Point<gpui::Pixels>) -> DropZone {
        let w = f32::from(bounds.size.width);
        let h = f32::from(bounds.size.height);
        let x = f32::from(position.x - bounds.origin.x);
        let y = f32::from(position.y - bounds.origin.y);
        let band = w.min(h) * DROP_EDGE_SHARE;
        let edges = [
            (DropZone::Left, x),
            (DropZone::Right, w - x),
            (DropZone::Top, y),
            (DropZone::Bottom, h - y),
        ];
        let (zone, distance) = edges
            .into_iter()
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap_or((DropZone::Center, 0.0));
        if distance < band {
            zone
        } else {
            DropZone::Center
        }
    }

    /// The split a side zone makes, and whether the new group goes after
    /// the one it was dropped on.
    fn split(self) -> Option<(SplitDirection, bool)> {
        match self {
            DropZone::Center => None,
            DropZone::Left => Some((SplitDirection::Vertical, false)),
            DropZone::Right => Some((SplitDirection::Vertical, true)),
            DropZone::Top => Some((SplitDirection::Horizontal, false)),
            DropZone::Bottom => Some((SplitDirection::Horizontal, true)),
        }
    }
}

/// How much of a group's shorter side, from each edge, drops as a split.
const DROP_EDGE_SHARE: f32 = 0.3;

/// Where a tab dragged over a strip would land, and the accent bar that
/// shows it: before tab `at` of `group`, at the tab boundary nearest the
/// pointer. The bar slides from where it was (`from`) to the boundary
/// (`x`, both window x) over `DROP_BAR_SLIDE` from `since`, so it is seen
/// to move between the tabs rather than jump.
#[derive(Debug, Clone, Copy)]
struct DropBar {
    group: u64,
    at: usize,
    x: f32,
    from: f32,
    since: Instant,
    /// The strip's left edge, which the bar's position is drawn from.
    strip_x: f32,
}

impl DropBar {
    /// Where the bar is drawn at `now`: on its way from `from` to `x`,
    /// easing out so it lands softly.
    fn shown_x(&self, now: Instant) -> f32 {
        let t =
            now.saturating_duration_since(self.since).as_secs_f32() / DROP_BAR_SLIDE.as_secs_f32();
        if t >= 1. {
            return self.x;
        }
        let eased = 1. - (1. - t) * (1. - t);
        self.from + (self.x - self.from) * eased
    }

    fn settled(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.since) >= DROP_BAR_SLIDE
    }
}

/// How long the drop bar takes to slide to a new boundary.
const DROP_BAR_SLIDE: Duration = Duration::from_millis(120);
/// The drop bar's width; it straddles the boundary between two tabs.
const DROP_BAR_WIDTH: f32 = 3.;

/// The ghost that follows the pointer while a tab is dragged: the tab's
/// title on a tab-shaped card whose top left corner sits at the cursor's
/// bottom right, as Obsidian's drag ghost does, so it never covers the
/// strip or the drop bar. gpui draws a drag's view where the dragged
/// element was under the pointer (the pointer less `grab`, the press's
/// offset into the tab), so the card is pushed back by that offset and then
/// past the cursor. The cursor itself is the grabbing hand for the drag.
struct TabDragPreview {
    title: SharedString,
    /// Where in the tab the press landed.
    grab: gpui::Point<gpui::Pixels>,
}

/// From the pointer's hotspot to the drag ghost's top left corner. The
/// grabbing hand's hotspot is at its centre, and the hand itself fills
/// about the middle half of a 24 px cursor's box, so this is where its
/// bottom right corner is.
const DRAG_GHOST_GAP: f32 = 8.;

impl Render for TabDragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        // The offset is padding on a wrapper rather than a margin on the
        // card: the layout drops a root element's margin, and the card is
        // the root of the drag's view.
        div()
            .pl(self.grab.x + px(DRAG_GHOST_GAP))
            .pt(self.grab.y + px(DRAG_GHOST_GAP))
            // The drag's view paints after everything else, and a cursor
            // asked for without a hitbox wins over the pointer the tab under
            // it asks for, so this is where the grabbing hand comes from.
            .child(canvas(
                |_, _, _| (),
                |_, _, window, _| window.set_window_cursor_style(CursorStyle::ClosedHand),
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(34.))
                    .px(px(10.))
                    .max_w(px(200.))
                    .rounded(px(6.))
                    .bg(theme.bg)
                    .border_1()
                    .border_color(theme.border)
                    .shadow_md()
                    .text_size(px(FONT_UI_SMALL))
                    .text_color(theme.text)
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(self.title.clone()),
            )
    }
}

/// The drag's on-screen stand-in: nothing, the divider moves in place.
struct NoDragPreview;

impl Render for NoDragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// The payload of a sidebar resize: which dock's inner edge is being dragged.
#[derive(Debug, Clone)]
struct DockDrag {
    left: bool,
}

/// The narrowest a sidebar can be dragged, and the narrowest `width`
/// `workspace.json` is believed about.
const MIN_SIDEBAR_WIDTH: f64 = 150.;

/// The margin around the content when Silica draws the window frame: the
/// shadow lives in it and the pointer resizes the window from it. Obsidian's
/// hidden frame measures 23 px at the sides, 15 above and 46 below on this
/// desktop; gpui's client inset is one value for all four sides, so this is
/// the side margin, and the shadow is shaped to fade out inside it.
const FRAME_SHADOW: Pixels = px(24.);

/// The frame shadow's gaussian sigma, which is what gpui's `blur_radius` is.
/// Obsidian's shadow fits sigma 13 with a 12 px downward offset; a shorter
/// one with a smaller offset is as dark at the content's edge and is gone
/// before the margin ends, where Obsidian's deeper bottom margin lets its own
/// tail run out. A tail cut off by the margin reads as a band around the
/// window.
const FRAME_SHADOW_BLUR: Pixels = px(11.);

/// How far the frame shadow sits below the content.
const FRAME_SHADOW_OFFSET: Pixels = px(6.);

/// The frame shadow's colour: black at 32 %, which is what Obsidian's comes
/// to at the sides.
const FRAME_SHADOW_COLOR: u32 = 0x00000052;

/// The radius of a window corner Silica draws itself. Obsidian's hidden
/// frame is rounded the same way on this desktop, about 10 px.
const WINDOW_RADIUS: Pixels = px(10.);

/// Obsidian's `--frame-left-space` on macOS: the traffic lights, which its
/// window options place at (19, 12), own the first 80 px of the title row,
/// less the ribbon's width while the ribbon shows. The header in the
/// window's top-left corner is padded by that past its usual 8 px.
const MAC_FRAME_LEFT_SPACE: f32 = 80.;

/// Which of the window's corners are rounded: each one that meets no tiled
/// or maximised edge, when the frame is Silica's. gpui clips children to
/// rectangles, so the frame's rounding alone is painted over by whatever
/// sits in a corner; every element that reaches one rounds itself with
/// [`WindowCorners::apply`], the way Zed's title bar and status bar do.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
struct WindowCorners {
    tl: bool,
    tr: bool,
    bl: bool,
    br: bool,
}

impl WindowCorners {
    fn of(window: &Window) -> WindowCorners {
        match window.window_decorations() {
            Decorations::Client { tiling } => WindowCorners {
                tl: !(tiling.top || tiling.left),
                tr: !(tiling.top || tiling.right),
                bl: !(tiling.bottom || tiling.left),
                br: !(tiling.bottom || tiling.right),
            },
            Decorations::Server => WindowCorners::default(),
        }
    }

    /// The left edge's corners only, for the ribbon or the left sidebar.
    fn left(self) -> WindowCorners {
        WindowCorners {
            tr: false,
            br: false,
            ..self
        }
    }

    /// The right edge's corners only, for the right sidebar.
    fn right(self) -> WindowCorners {
        WindowCorners {
            tl: false,
            bl: false,
            ..self
        }
    }

    fn apply<T: Styled>(self, mut el: T) -> T {
        if self.tl {
            el = el.rounded_tl(WINDOW_RADIUS);
        }
        if self.tr {
            el = el.rounded_tr(WINDOW_RADIUS);
        }
        if self.bl {
            el = el.rounded_bl(WINDOW_RADIUS);
        }
        if self.br {
            el = el.rounded_br(WINDOW_RADIUS);
        }
        el
    }
}

/// A 28 by 24 icon button of the kind Obsidian calls a clickable icon:
/// 4 px above and below the icon, 6 px either side, a 4 px radius and a
/// wash behind it on hover.
/// Makes the sandbox vault at `root`: a welcome note that links to a
/// second, so the editor, the links and the graph have something to show.
/// The folder is new each time, so nothing already there is written over.
fn write_sandbox(root: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(root)?;
    std::fs::write(
        root.join("Welcome.md"),
        "---\ntags:\n  - sandbox\n---\n# Welcome to the sandbox\n\nThis vault is a scratch folder: try things here and nothing you keep is \
touched. Link to [[Another note]], press Ctrl+O to switch notes, Ctrl+P \
for the command palette, and Ctrl+E to flip between editing and reading.\n\n\
- [ ] A task to tick\n- A list item with **bold** and *italic* text\n\n\
> [!tip] Callouts\n> Obsidian's callouts render here too.\n",
    )?;
    std::fs::write(
        root.join("Another note.md"),
        "# Another note\n\nIt links back to [[Welcome]], which the backlinks pane shows.\n",
    )
}

/// What an empty tab offers, in Obsidian's order: the element's id, the
/// label on the pill and the command it runs. Obsidian draws each as a
/// `div.text-icon-button.tappable.mod-pill.empty-state-action`; what is
/// observed and has to match is the text and that the pill runs the
/// command it names (see [[Specification#The empty tab]]).
const EMPTY_TAB_ACTIONS: [(&str, &str, &str); 4] = [
    ("empty-new-note", "New note", "file-explorer:new-file"),
    ("empty-new-base", "New base", "bases:new-file"),
    ("empty-new-canvas", "New canvas", "canvas:new-file"),
    ("empty-go-to-file", "Go to file", "switcher:open"),
];

/// The path a file or folder at `from` takes when the name a rename field
/// holds becomes `value`, or the reason the filesystem would refuse it.
///
/// A rename field holds the stem, so the name that will be asked for does
/// not exist yet and both have to be checked, or an empty field is refused
/// for starting with a dot and a very long one reaches `rename` and comes
/// back as an errno and an absolute path. The rename prompt's box and the
/// view header's inline field both come through here, so there is one
/// validator and one way a name becomes a path.
fn renamed_path(from: &str, value: &str, is_folder: bool) -> Result<String, String> {
    let folder = from.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let name = from.rsplit('/').next().unwrap_or(from);
    let ext = if is_folder {
        None
    } else {
        name.rsplit_once('.')
            .map(|(_, e)| e)
            .filter(|_| basename(name) != name)
    };
    validate_renamed_file(value, ext).map_err(|err| err.to_string())?;
    let new_name = match ext {
        Some(ext) => format!("{value}.{ext}"),
        None => value.to_string(),
    };
    Ok(if folder.is_empty() {
        new_name
    } else {
        format!("{folder}/{new_name}")
    })
}

/// Whether the view header carries the mode switch. An empty tab's header
/// holds the two navigation buttons and nothing else -- no mode switch,
/// since there is nothing to read or edit (see
/// [[Specification#The empty tab]]) -- and the graph has its own toggle.
fn header_shows_mode_switch(current: &Option<String>, mode: ViewMode) -> bool {
    current.is_some() && mode != ViewMode::Graph
}

/// What the status bar reads while an empty tab is active, item by item.
/// It is not hidden there (observed, [[Specification#The empty tab]]): the
/// items stay and read as zero.
const EMPTY_TAB_STATUS: [&str; 3] = ["0 properties", "0 words", "0 characters"];

/// One pill of the empty tab.
struct EmptyAction {
    id: &'static str,
    label: &'static str,
    /// The command's first hotkey as the Hotkeys tab writes it, or `None`
    /// for a command with none: `New note Ctrl + N` carries one, `New base`
    /// does not. It comes from the registry, so `hotkeys.json` applies and
    /// a rebound command says so.
    hotkey: Option<String>,
    command: &'static str,
}

impl EmptyAction {
    /// The pill's text: the label, then the hotkey when there is one.
    fn text(&self) -> String {
        match &self.hotkey {
            Some(keys) => format!("{} {keys}", self.label),
            None => self.label.to_string(),
        }
    }
}

/// The pills an empty tab draws. A command the registry does not have is
/// left out rather than drawn dead: the registry holds the commands of the
/// core plugins that are on, so turning Bases off takes "New base" with it.
fn empty_tab_actions(registry: &CommandRegistry) -> Vec<EmptyAction> {
    EMPTY_TAB_ACTIONS
        .iter()
        .filter_map(|(id, label, command)| {
            let known = registry.get(command)?;
            Some(EmptyAction {
                id,
                label,
                hotkey: known.hotkeys.first().map(hotkey_label),
                command,
            })
        })
        .collect()
}

fn icon_button(id: &'static str, name: &str, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_shrink_0()
        .w(px(28.))
        .h(px(24.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .cursor_pointer()
        .hover(|s| s.bg(theme.bg_alt))
        .child(icon(name, ICON_SIZE, theme.text_muted))
}

/// Makes `area` a handle on the window itself when the frame is Silica's:
/// a press drags the window, a double click maximises or restores it and a
/// right click opens the desktop's window menu, as the empty part of
/// Obsidian's title row does.
///
/// Windows reaches all three through the hit test instead: naming the area
/// the window's caption is what `WindowControlArea::Drag` does, and the
/// system then drags, maximises and opens its menu itself. Asking for both
/// would be worse than asking for one — `start_window_move` is a no-op
/// there, and a listener that swallowed the press would stop the system
/// dragging at all.
/// A press on a macOS drag area that has not moved yet; one window, one
/// pointer, so a flag is enough.
static MAC_DRAG_PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn window_handle(area: gpui::Stateful<gpui::Div>, frameless: bool) -> gpui::Stateful<gpui::Div> {
    if !frameless {
        return area;
    }
    if cfg!(target_os = "windows") {
        return area.window_control_area(WindowControlArea::Drag);
    }
    if cfg!(target_os = "macos") {
        // AppKit's drag session swallows the click it starts from, so the
        // drag begins on the first move after a press and a double click
        // still arrives as one: it does what the desktop's "Double-click a
        // window's title bar to" setting says. macOS has no window menu.
        return area
            .on_mouse_down(MouseButton::Left, |_, _, _| {
                MAC_DRAG_PENDING.store(true, std::sync::atomic::Ordering::Relaxed)
            })
            .on_mouse_up(MouseButton::Left, |_, _, _| {
                MAC_DRAG_PENDING.store(false, std::sync::atomic::Ordering::Relaxed)
            })
            .on_mouse_down_out(|_, _, _| {
                MAC_DRAG_PENDING.store(false, std::sync::atomic::Ordering::Relaxed)
            })
            .on_mouse_move(|_, window, _| {
                if MAC_DRAG_PENDING.swap(false, std::sync::atomic::Ordering::Relaxed) {
                    window.start_window_move();
                }
            })
            .on_click(|e, window, _| {
                if e.click_count() == 2 {
                    window.titlebar_double_click();
                }
            });
    }
    area.on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
        .on_click(|e, window, _| {
            if e.click_count() == 2 {
                window.zoom_window();
            }
        })
        .on_mouse_down(MouseButton::Right, |e, window, _| {
            window.show_window_menu(e.position)
        })
}

/// Which edge or corner of the window `pos` is over, when it is in the
/// shadow margin of one that is not tiled on that side.
fn resize_edge(
    pos: gpui::Point<Pixels>,
    size: gpui::Size<Pixels>,
    tiling: Tiling,
) -> Option<ResizeEdge> {
    let m = FRAME_SHADOW;
    let top = pos.y < m && !tiling.top;
    let bottom = pos.y > size.height - m && !tiling.bottom;
    let left = pos.x < m && !tiling.left;
    let right = pos.x > size.width - m && !tiling.right;
    Some(match (top, bottom, left, right) {
        (true, _, true, _) => ResizeEdge::TopLeft,
        (true, _, _, true) => ResizeEdge::TopRight,
        (true, _, _, _) => ResizeEdge::Top,
        (_, true, true, _) => ResizeEdge::BottomLeft,
        (_, true, _, true) => ResizeEdge::BottomRight,
        (_, true, _, _) => ResizeEdge::Bottom,
        (_, _, true, _) => ResizeEdge::Left,
        (_, _, _, true) => ResizeEdge::Right,
        _ => return None,
    })
}

/// Whether Silica draws the window's title row itself, rather than the
/// desktop drawing a title bar above it.
///
/// On Linux this is what the compositor granted, which is not always what
/// was asked for: `Decorations::Client` means the frame is ours. Windows
/// and macOS answer `Server` either way — neither has a notion of handing
/// the frame over — and the title bar is instead gone whenever the window
/// was opened with a transparent one, so there the answer is what was
/// asked for. On macOS the title row then holds the desktop's own traffic
/// lights rather than window buttons of Silica's; see `mac_frame`.
fn frameless(window: &Window) -> bool {
    if cfg!(target_os = "windows") || cfg!(target_os = "macos") {
        crate::hidden_frame()
    } else {
        matches!(window.window_decorations(), Decorations::Client { .. })
    }
}

/// Silica's own window frame, for a window the desktop was asked to leave
/// bare: a margin of [`FRAME_SHADOW`] around the content holds a soft
/// shadow and the resize handles, and the content gets a one pixel border.
/// A tiled or maximised edge loses its margin, border and shadow, so a
/// snapped window meets the screen edge without a gap. With the desktop's
/// own frame the content is passed through untouched.
fn client_frame(
    content: gpui::AnyElement,
    theme: &Theme,
    window: &mut Window,
) -> gpui::Stateful<gpui::Div> {
    let decorations = window.window_decorations();
    let tiling = match decorations {
        Decorations::Client { tiling } => tiling,
        Decorations::Server => Tiling::default(),
    };
    let client = matches!(decorations, Decorations::Client { .. });
    window.set_client_inset(if client { FRAME_SHADOW } else { px(0.) });
    let border = theme.border;
    let shadow: Hsla = rgba(FRAME_SHADOW_COLOR).into();
    let corners = WindowCorners::of(window);
    div()
        .id("window-backdrop")
        .size_full()
        .bg(transparent_black())
        .map(|d| corners.apply(d))
        .when(client, |d| {
            d.when(!tiling.top, |d| d.pt(FRAME_SHADOW))
                .when(!tiling.bottom, |d| d.pb(FRAME_SHADOW))
                .when(!tiling.left, |d| d.pl(FRAME_SHADOW))
                .when(!tiling.right, |d| d.pr(FRAME_SHADOW))
                .child(
                    canvas(
                        |_, window, _| {
                            window.insert_hitbox(
                                gpui::Bounds::new(
                                    point(px(0.), px(0.)),
                                    window.window_bounds().get_bounds().size,
                                ),
                                HitboxBehavior::Normal,
                            )
                        },
                        move |_, hitbox, window, _| {
                            let size = window.window_bounds().get_bounds().size;
                            let Some(edge) = resize_edge(window.mouse_position(), size, tiling)
                            else {
                                return;
                            };
                            let style = match edge {
                                ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
                                ResizeEdge::Left | ResizeEdge::Right => {
                                    CursorStyle::ResizeLeftRight
                                }
                                ResizeEdge::TopLeft | ResizeEdge::BottomRight => {
                                    CursorStyle::ResizeUpLeftDownRight
                                }
                                ResizeEdge::TopRight | ResizeEdge::BottomLeft => {
                                    CursorStyle::ResizeUpRightDownLeft
                                }
                            };
                            window.set_cursor_style(style, &hitbox);
                        },
                    )
                    .size_full()
                    .absolute(),
                )
                .on_mouse_move(|_, window, _| window.refresh())
                .on_mouse_down(MouseButton::Left, move |e, window, _| {
                    let size = window.window_bounds().get_bounds().size;
                    if let Some(edge) = resize_edge(e.position, size, tiling) {
                        window.start_window_resize(edge);
                    }
                })
        })
        .child(
            div()
                .size_full()
                .cursor(CursorStyle::Arrow)
                .bg(theme.bg)
                .map(|d| corners.apply(d))
                .when(client, |d| {
                    d.border_color(border)
                        .when(!tiling.top, |d| d.border_t_1())
                        .when(!tiling.bottom, |d| d.border_b_1())
                        .when(!tiling.left, |d| d.border_l_1())
                        .when(!tiling.right, |d| d.border_r_1())
                        .when(!tiling.is_tiled(), |d| {
                            d.shadow(vec![BoxShadow {
                                color: shadow,
                                blur_radius: FRAME_SHADOW_BLUR,
                                spread_radius: px(0.),
                                offset: point(px(0.), FRAME_SHADOW_OFFSET),
                            }])
                        })
                })
                // The margin's own listeners see only what the content
                // lets through, so the content keeps the pointer's moves.
                .on_mouse_move(|_, _, cx| cx.stop_propagation())
                .child(content),
        )
}

/// How many closed tabs `workspace:undo-close-pane` can bring back.
const CLOSED_TABS_KEPT: usize = 20;

/// How many paths `workspace.json`'s `lastOpenFiles` keeps.
const LAST_OPEN_FILES_KEPT: usize = 10;

pub struct Workspace {
    focus_handle: FocusHandle,
    /// Set when something changed what the main area shows and the keyboard
    /// has to follow it; [`Workspace::render`] is where it is honoured.
    refocus_main: bool,
    state: Option<Arc<VaultState>>,
    vault_name: SharedString,
    /// Whether the vault being loaded goes on the switcher's list: a
    /// sandbox does not.
    record_vault: bool,
    /// The vault switcher's menu, dropped up from the sidebar's footer.
    vault_menu_open: bool,
    /// Whether the pointer is over the switcher row. The menu closes on a
    /// mouse *down* outside itself, which a click on the row is: without
    /// this the press would close the menu and the click that ends the
    /// same press would open it again, leaving it stuck open under a
    /// flicker. The out-handler leaves the row's own press to the row.
    vault_switcher_hovered: bool,
    /// The group whose tab-list menu (the strip's `chevron-down`) is open.
    tab_list_menu: Option<u64>,
    /// The Help window while one is up; a stale handle means it was closed.
    help_window: Option<gpui::WindowHandle<HelpWindow>>,
    /// The vault manager while one is up, the same way.
    vault_window: Option<gpui::WindowHandle<VaultManager>>,
    loading: Option<SharedString>,
    explorer: Entity<FileExplorer>,
    search: Entity<SearchPane>,
    bookmarks: Entity<BookmarksPane>,
    switcher: Entity<QuickSwitcher>,
    chooser: Entity<Chooser>,
    /// What the chooser's pick means while it is open.
    chooser_purpose: ChooserPurpose,
    /// A canvas asked for the note or media picker; the next render opens
    /// it, with the window the chooser needs to take focus.
    pending_canvas_pick: Option<crate::canvas::FilePick>,
    palette: Entity<CommandPalette>,
    settings: Entity<SettingsView>,
    prompt: Entity<Prompt>,
    copy_results: Entity<CopyResults>,
    pending: Option<PendingOp>,
    modal: Modal,
    /// Command ids run this session, most recent first.
    recent_commands: Vec<String>,
    /// The notes' fold records, kept outside the vault; opened for the
    /// vault's root the first time a note needs one.
    fold_store: Option<(PathBuf, fold::FoldStore)>,
    left_visible: bool,
    right_visible: bool,
    /// Each sidebar's width in logical pixels, `workspace.json`'s
    /// `left.width` and `right.width`; a new vault's is 300.
    left_width: f64,
    right_width: f64,
    /// Whether the window has Silica's own frame rather than the desktop's,
    /// read from the window at each render for the renderers that place the
    /// window buttons and the drag areas.
    frameless: bool,
    /// Whether the window fills the screen, read with `frameless`: the
    /// macOS frame drops the traffic lights' space then.
    fullscreen: bool,
    /// The window corners rounded this frame, read at the top of `render`.
    corners: WindowCorners,
    left_tab: LeftTab,
    reading: Entity<ReadingView>,
    editor: Entity<Editor>,
    /// The active group's canvas view, like `editor` and `reading`; it
    /// shows the open file when that is a `.canvas`.
    canvas: Entity<CanvasView>,
    /// The active group's Bases view, the same way: it shows the open file
    /// when that is a `.base`.
    bases: Entity<BasesView>,
    /// The active group's PDF view, the same way: it shows the open file
    /// when that is a `.pdf`.
    pdf: Entity<PdfView>,
    graph: Entity<GraphView>,
    /// The mode to return to when the graph closes.
    mode_before_graph: ViewMode,
    mode: ViewMode,
    current: Option<String>,
    /// The note whose file went away while its buffer was still on screen.
    /// The text stays — it may hold edits that exist nowhere else — but it
    /// is no longer backed by a file, so the main area says so and nothing
    /// writes it back.
    detached: Option<SharedString>,
    history: Vec<HistoryEntry>,
    history_ix: usize,
    /// The main area's tabs; the active one's fields are the live ones above.
    tabs: Vec<Tab>,
    active_tab: usize,
    /// Tabs closed this session with the index they had, oldest first.
    closed_tabs: Vec<(usize, Tab)>,
    /// Every tab group, the active one's slot holding what it last stashed.
    groups: Vec<TabGroup>,
    active_group: usize,
    /// How the groups are split, as `workspace.json`'s `main` records it.
    layout_tree: Layout,
    next_group_id: u64,
    /// Group ids, most recently active first: where focus goes when a group
    /// closes, and which group a focus command lands in.
    group_mru: Vec<u64>,
    /// A tab is being dragged, so every group paints its drop zones. Set
    /// when the drag starts and cleared when the next frame finds no drag
    /// under way, however the drag ended.
    dragging_tab: bool,
    /// Which group's content the dragged tab is over, and where in it.
    tab_drop: Option<(u64, DropZone)>,
    /// The accent bar between two tabs of the strip the dragged tab is
    /// over, where it would land.
    drop_bar: Option<DropBar>,
    /// Where each tab was on the last drag move, by group and index: what
    /// a strip measures the pointer against to find the nearest boundary.
    tab_bounds: HashMap<(u64, usize), gpui::Bounds<gpui::Pixels>>,
    /// The keyboard while the view-header title is being typed into. It is
    /// its own handle rather than the workspace's, so the title only takes
    /// the keys while a click has put the caret in it.
    title_focus: FocusHandle,
    /// The header title being renamed in place, if any.
    title_edit: Option<TitleEdit>,
    /// Where each group's header title was painted and how it was shaped,
    /// left by the last frame: what a click on a title is measured against.
    title_layout: HashMap<u64, TitleLayout>,
    /// `workspace.json` as it was read at load, `None` when the vault has
    /// none; what the quit write is merged into.
    layout: Option<Value>,
    /// A `workspace.json` was read, so the startup flags leave the first
    /// note unopened the way Obsidian does.
    layout_restored: bool,
    /// `lastOpenFiles`: the notes opened, most recent first.
    last_open_files: Vec<String>,
    /// What the window's title bar says, set only when it changes.
    window_title: SharedString,
    /// The next link followed opens in a new tab: a Ctrl+click, or
    /// `editor:open-link-in-new-leaf`. Taken by [`Workspace::follow_link`].
    link_in_new_tab: bool,
    right_tab: RightTab,
    /// The open note's headings, which the Outline pane draws as a tree.
    outline: Vec<panes::Heading>,
    outline_state: OutlinePaneState,
    outline_filter: Entity<TextInput>,
    /// The Backlinks pane: Linked mentions, then Unlinked mentions, which
    /// are filled in when the background search for them lands.
    backlink_sections: Vec<Section>,
    /// The Outgoing links pane: Links, then Unlinked mentions.
    outgoing_sections: Vec<Section>,
    backlink_state: BacklinkState,
    outgoing_state: OutgoingState,
    /// Where the pointer was when a pane's sort button was clicked: the
    /// menu opens there, its top-right corner at the click, as Obsidian's does.
    sort_menu_at: gpui::Point<Pixels>,
    /// The Backlinks pane's search filter (`searchQuery`).
    backlink_filter: Entity<TextInput>,
    /// Every tag in the vault as a tree, sorted the way the Tags pane is.
    tag_tree: Vec<panes::TagNode>,
    tag_state: TagPaneState,
    tag_filter: Entity<TextInput>,
    /// Every property key in the vault, at the cache's frequency order.
    property_entries: Vec<PropertyEntry>,
    property_state: PropertyPaneState,
    property_filter: Entity<TextInput>,
    /// Matcher over every note name and alias, rebuilt when files change.
    name_finder: Option<Arc<Finder>>,
    mention_generation: u64,
    /// What the status bar says about the vault and the open file: it is
    /// true until something makes it untrue, so nothing expires it.
    status: SharedString,
    notices: Notices,
    words: usize,
    chars: usize,
    _watcher: Option<watch::Watcher>,
    /// Pending autosave, two seconds after the last edit.
    save_task: Option<Task<()>>,
    /// Whether the startup flags have been applied. A client driving the
    /// control socket needs to know that the window has stopped arranging
    /// itself, not just that the vault finished indexing.
    started: bool,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for Workspace {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// What the workspace listens to on a group's editor and reading view. A
/// click in a group makes it the active one before the view handles the
/// click, so by the time one of these arrives the views that sent it are
/// the live ones.
fn view_subscriptions(
    editor: &Entity<Editor>,
    reading: &Entity<ReadingView>,
    canvas: &Entity<CanvasView>,
    bases: &Entity<BasesView>,
    pdf: &Entity<PdfView>,
    cx: &mut Context<Workspace>,
) -> Vec<Subscription> {
    vec![
        cx.subscribe(pdf, |this, _, event: &PdfEvent, cx| match event {
            PdfEvent::Report(message) => this.report(message.clone(), cx),
        }),
        // A row of a base names a file; activating it opens that file, the
        // same route a link in the reading view takes.
        cx.subscribe(bases, |this, _, event: &BasesEvent, cx| match event {
            BasesEvent::Open(path) => this.open_file(path.clone(), None, cx),
            BasesEvent::Edited => this.schedule_save(cx),
            BasesEvent::Report(message) => this.report(message.clone(), cx),
            BasesEvent::SetProperty { path, key, value } => {
                this.set_note_property(path.clone(), key.clone(), value.clone(), cx)
            }
        }),
        cx.subscribe(
            canvas,
            |this, canvas, event: &CanvasEvent, cx| match event {
                CanvasEvent::Edited => this.schedule_save(cx),
                CanvasEvent::OpenLink(target) => this.follow_link(target.clone(), cx),
                CanvasEvent::Viewport(view_state) => {
                    this.record_canvas_view_state(&canvas, view_state.clone(), cx)
                }
                // The chooser needs the window to take focus, which this
                // subscription does not have: the next render opens it.
                CanvasEvent::PickFile(kind) => {
                    this.pending_canvas_pick = Some(*kind);
                    cx.notify();
                }
            },
        ),
        cx.subscribe(reading, |this, _, event: &ReadingEvent, cx| match event {
            ReadingEvent::OpenLink(target) => this.follow_link(target.clone(), cx),
            ReadingEvent::OpenLinkInNewTab(target) => {
                this.link_in_new_tab = true;
                this.follow_link(target.clone(), cx);
            }
            _ => {}
        }),
        cx.subscribe(reading, |this, _, event: &TaskToggled, cx| {
            this.toggle_task(&event.path, event.marker, cx)
        }),
        cx.subscribe(reading, |this, _, event: &ReadingEvent, cx| match event {
            &ReadingEvent::PropertyChanged { ref path, ref changes } => {
                this.change_property(path, changes, cx)
            }
            _ => {}
        }),
        cx.subscribe(editor, |this, _, event: &EditorEvent, cx| match event {
            EditorEvent::Edited => {
                this.schedule_save(cx);
                this.refresh_outline_from_buffer(cx);
            }
            EditorEvent::OpenLink(target) => this.follow_link(target.clone(), cx),
            EditorEvent::OpenLinkInNewTab(target) => {
                this.link_in_new_tab = true;
                this.follow_link(target.clone(), cx);
            }
            EditorEvent::PropertyTypeAssigned { key, ty } => {
                this.assign_property_type(key.clone(), *ty, cx)
            }
            EditorEvent::Reported(message) => this.report(message.clone(), cx),
            EditorEvent::FilesChanged(changes) => {
                this.after_changes(changes.clone(), cx);
            }
        }),
    ]
}

impl Workspace {
    pub fn new(root: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Workspace {
        let explorer = cx.new(FileExplorer::new);
        let search = cx.new(SearchPane::new);
        let backlink_filter = cx.new(pane_filter_input);
        let tag_filter = cx.new(pane_filter_input);
        let property_filter = cx.new(pane_filter_input);
        let outline_filter = cx.new(pane_filter_input);
        let bookmarks = cx.new(BookmarksPane::new);
        let switcher = cx.new(QuickSwitcher::new);
        let chooser = cx.new(Chooser::new);
        let palette = cx.new(CommandPalette::new);
        let settings = cx.new(SettingsView::new);
        let prompt = cx.new(Prompt::new);
        let copy_results = cx.new(CopyResults::new);
        let reading = cx.new(ReadingView::new);
        let editor = cx.new(|cx| Editor::new(window, cx));
        let canvas = cx.new(CanvasView::new);
        let bases = cx.new(BasesView::new);
        let pdf = cx.new(PdfView::new);
        let graph = cx.new(GraphView::new);
        let subscriptions = vec![
            // The filter is applied as the pane is drawn; a keystroke only
            // has to ask for a draw.
            cx.subscribe(&backlink_filter, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Changed) {
                    cx.notify();
                }
            }),
            // Escape in the Tags filter is not the pane's: Obsidian hands
            // the keyboard to the editor and leaves the rows filtered.
            cx.subscribe_in(
                &tag_filter,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Changed => cx.notify(),
                    InputEvent::Cancel => this.focus_main(window, cx),
                    _ => {}
                },
            ),
            // Escape in the other two hides the box and clears the query.
            cx.subscribe_in(
                &property_filter,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Changed => cx.notify(),
                    InputEvent::Cancel => this.hide_pane_search(RightTab::Properties, window, cx),
                    _ => {}
                },
            ),
            cx.subscribe_in(
                &outline_filter,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Changed => cx.notify(),
                    InputEvent::Cancel => this.hide_pane_search(RightTab::Outline, window, cx),
                    _ => {}
                },
            ),
            // The Outline marks the heading holding the cursor, or the one
            // at the top of the reading view: it follows both views.
            cx.observe(&editor, |this, _, cx| this.follow_heading(cx)),
            cx.observe(&reading, |this, _, cx| this.follow_heading(cx)),
            cx.subscribe_in(
                &graph,
                window,
                |this, _, event: &GraphEvent, window, cx| match event {
                    GraphEvent::Open(path) => {
                        let back = this.mode_before_graph;
                        this.set_mode(back, window, cx);
                        this.open_file(path.clone(), None, cx);
                    }
                },
            ),
            cx.subscribe_in(
                &explorer,
                window,
                |this, _, event: &ExplorerEvent, window, cx| match event {
                    ExplorerEvent::Open(path) => {
                        // A row's click or Enter hands the keyboard to the
                        // note it opened, as Obsidian's does; the tree
                        // keeps its ring for the next reveal.
                        this.open_file(path.clone(), None, cx);
                        this.focus_main(window, cx);
                    }
                    ExplorerEvent::OpenInNewTab(path) => {
                        this.open_in_new_tab(path.clone(), None, cx)
                    }
                    ExplorerEvent::NewNote { folder } => this.new_note(folder, window, cx),
                    ExplorerEvent::NewNoteFromHeader => {
                        this.run_command("file-explorer:new-file", window, cx)
                    }
                    ExplorerEvent::NewFolder { parent } => {
                        this.start_new_folder(parent, window, cx)
                    }
                    ExplorerEvent::Rename(path) => this.start_rename(path, window, cx),
                    ExplorerEvent::Move(path) => this.start_move(path, window, cx),
                    ExplorerEvent::MoveToFolder { src, dest_folder } => {
                        let name = src.rsplit('/').next().unwrap_or(&src);
                        let to = if dest_folder.is_empty() {
                            name.to_string()
                        } else {
                            format!("{dest_folder}/{name}")
                        };
                        this.rename_with_links(&src, &to, window, cx);
                    }
                    ExplorerEvent::Duplicate(path) => this.duplicate(path, cx),
                    ExplorerEvent::Delete(path) => this.start_delete(path, window, cx),
                    ExplorerEvent::Bookmark(path) => this.start_bookmark_path(path, window, cx),
                    ExplorerEvent::NewCanvas { folder } => this.new_canvas(folder, cx),
                    ExplorerEvent::NewBase { folder } => this.new_base(folder, cx),
                    ExplorerEvent::OpenToRight(path) => {
                        this.open_to_right(path.clone(), window, cx)
                    }
                    ExplorerEvent::CopyPath(path) => this.copy_path(path, cx),
                    ExplorerEvent::OpenInDefaultApp(path) => this.open_in_default_app(path, cx),
                    ExplorerEvent::SearchInFolder(folder) => {
                        this.search_in_folder(folder, window, cx)
                    }
                    ExplorerEvent::LeaveFocus => this.focus_main(window, cx),
                },
            ),
            cx.subscribe_in(
                &bookmarks,
                window,
                |this, _, event: &BookmarkEvent, window, cx| match event {
                    BookmarkEvent::Open { path, subpath } => {
                        let link = format!("{path}{}", subpath.as_deref().unwrap_or(""));
                        let parsed = silica_markdown::parse_linktext(&link);
                        this.open_file(path.clone(), parsed.subpath, cx);
                    }
                    BookmarkEvent::Search(query) => this.show_search(query, Some(window), cx),
                    BookmarkEvent::Graph => this.show_graph(false, window, cx),
                    BookmarkEvent::Folder(path) => {
                        this.left_tab = LeftTab::Files;
                        let path = path.clone();
                        this.explorer.update(cx, |e, cx| e.reveal_folder(&path, cx));
                        cx.notify();
                    }
                    BookmarkEvent::Url(url) => cx.open_url(url),
                    BookmarkEvent::AddCurrent => this.start_bookmark_current_view(window, cx),
                    BookmarkEvent::NewGroup => this.start_new_bookmark_group(window, cx),
                    BookmarkEvent::Remove(at) => {
                        let at = at.clone();
                        this.change_bookmarks(cx, move |b| {
                            b.remove_at(&at);
                        });
                    }
                },
            ),
            cx.subscribe_in(
                &prompt,
                window,
                |this, _, event: &PromptEvent, window, cx| match event {
                    PromptEvent::Submit(value) => this.prompt_submitted(value, window, cx),
                    PromptEvent::Secondary => this.prompt_secondary(window, cx),
                    PromptEvent::Cancel => {
                        this.pending = None;
                        this.close_modal(window, cx);
                    }
                },
            ),
            cx.subscribe_in(
                &copy_results,
                window,
                |this, _, event: &CopyResultsEvent, window, cx| match event {
                    CopyResultsEvent::Copy(text) => {
                        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                        this.report("Results copied to your clipboard.", cx);
                    }
                    CopyResultsEvent::Close => this.close_modal(window, cx),
                },
            ),
            cx.subscribe_in(
                &search,
                window,
                |this, _, event: &SearchEvent, window, cx| match event {
                    SearchEvent::Open(path, offset) => {
                        this.open_file(path.clone(), None, cx);
                        if let Some(offset) = *offset {
                            this.reveal_offset(offset, cx);
                        }
                    }
                    SearchEvent::Bookmark(query) => {
                        this.start_bookmark(BookmarkDraft::Search(query.clone()), window, cx);
                    }
                    SearchEvent::FilesChanged(changes) => {
                        this.after_changes(changes.clone(), cx);
                    }
                    SearchEvent::CopyResults(entries) => {
                        this.open_copy_results(entries.clone(), window, cx);
                    }
                    SearchEvent::Dismiss => this.focus_main(window, cx),
                    SearchEvent::StateChanged => {}
                },
            ),
            cx.subscribe_in(
                &switcher,
                window,
                |this, _, event: &SwitcherEvent, window, cx| match event {
                    SwitcherEvent::Open(path) => {
                        this.close_modal(window, cx);
                        this.open_file(path.clone(), None, cx);
                    }
                    SwitcherEvent::Create(name) => {
                        this.close_modal(window, cx);
                        this.create_from_link(name, window, cx);
                    }
                    SwitcherEvent::Close => this.close_modal(window, cx),
                },
            ),
            cx.subscribe_in(
                &palette,
                window,
                |this, _, event: &PaletteEvent, window, cx| match event {
                    PaletteEvent::Run(id) => {
                        this.close_modal(window, cx);
                        let id = id.clone();
                        this.run_command(&id, window, cx);
                    }
                    PaletteEvent::Close => this.close_modal(window, cx),
                },
            ),
            cx.subscribe_in(
                &settings,
                window,
                |this, _, event: &SettingsEvent, window, cx| match event {
                    SettingsEvent::Close => this.close_modal(window, cx),
                    SettingsEvent::Changed(change) => {
                        this.change_setting(change.as_ref().clone(), window, cx)
                    }
                },
            ),
            cx.subscribe_in(
                &chooser,
                window,
                |this, _, event: &ChooserEvent, window, cx| match event {
                    ChooserEvent::Chosen(value) => match this.chooser_purpose {
                        ChooserPurpose::Template => this.insert_template(value.clone(), window, cx),
                        ChooserPurpose::CanvasFile => {
                            this.modal = Modal::None;
                            this.canvas
                                .update(cx, |c, cx| c.add_file_card_at_center(value, cx));
                            this.focus_main(window, cx);
                            cx.notify();
                        }
                        ChooserPurpose::CanvasGroup => {
                            this.modal = Modal::None;
                            this.canvas.update(cx, |c, cx| c.jump_to_group(value, cx));
                            this.focus_main(window, cx);
                            cx.notify();
                        }
                        ChooserPurpose::BasesView => {
                            this.modal = Modal::None;
                            // The row carries the view's index into
                            // `base.views`, which is what the Bases view
                            // switches on; a name would not do, since
                            // nothing stops two views sharing one.
                            if let Ok(ix) = value.parse::<usize>() {
                                this.bases.update(cx, |b, cx| b.switch_view(ix, cx));
                            }
                            this.focus_main(window, cx);
                            cx.notify();
                        }
                    },
                    ChooserEvent::Close => {
                        this.modal = Modal::None;
                        // Whatever the main area paints, not the editor:
                        // its handle is outside the dispatch tree in
                        // reading view and would swallow every key.
                        this.focus_main(window, cx);
                        cx.notify();
                    }
                },
            ),
        ];
        let mut subscriptions = subscriptions;
        subscriptions.extend(view_subscriptions(
            &editor, &reading, &canvas, &bases, &pdf, cx,
        ));
        let groups = vec![TabGroup::new(
            0,
            editor.clone(),
            reading.clone(),
            canvas.clone(),
            bases.clone(),
            pdf.clone(),
            ViewMode::Reading,
        )];
        // Closing the window must not lose edits waiting for the autosave.
        let this = cx.entity().downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            if let Some(ws) = this.upgrade() {
                ws.update(cx, |ws, cx| ws.on_quit(cx));
            }
            true
        });
        let mut ws = Workspace {
            focus_handle: cx.focus_handle(),
            refocus_main: false,
            state: None,
            vault_name: "No vault".into(),
            record_vault: true,
            vault_menu_open: false,
            vault_switcher_hovered: false,
            tab_list_menu: None,
            help_window: None,
            vault_window: None,
            loading: None,
            explorer,
            search,
            bookmarks,
            switcher,
            chooser,
            chooser_purpose: ChooserPurpose::default(),
            pending_canvas_pick: None,
            palette,
            settings,
            prompt,
            copy_results,
            pending: None,
            modal: Modal::None,
            recent_commands: Vec::new(),
            fold_store: None,
            left_visible: true,
            right_visible: true,
            left_width: SIDEBAR_WIDTH as f64,
            right_width: SIDEBAR_WIDTH as f64,
            frameless: false,
            fullscreen: false,
            corners: WindowCorners::default(),
            left_tab: LeftTab::Files,
            reading,
            editor,
            canvas,
            bases,
            pdf,
            graph,
            mode_before_graph: ViewMode::Reading,
            mode: ViewMode::Reading,
            current: None,
            detached: None,
            history: Vec::new(),
            history_ix: 0,
            tabs: vec![Tab::empty(ViewMode::Reading)],
            active_tab: 0,
            closed_tabs: Vec::new(),
            groups,
            active_group: 0,
            layout_tree: Layout::split(SplitDirection::Vertical, vec![Layout::Group(0)]),
            next_group_id: 1,
            group_mru: vec![0],
            dragging_tab: false,
            tab_drop: None,
            drop_bar: None,
            tab_bounds: HashMap::new(),
            title_focus: cx.focus_handle(),
            title_edit: None,
            title_layout: HashMap::new(),
            layout: None,
            layout_restored: false,
            last_open_files: Vec::new(),
            window_title: SharedString::default(),
            link_in_new_tab: false,
            right_tab: RightTab::Backlinks,
            outline: Vec::new(),
            outline_state: OutlinePaneState::default(),
            outline_filter,
            backlink_sections: Vec::new(),
            outgoing_sections: Vec::new(),
            backlink_state: BacklinkState::default(),
            outgoing_state: OutgoingState::default(),
            sort_menu_at: point(px(0.), px(0.)),
            backlink_filter,
            tag_tree: Vec::new(),
            tag_state: TagPaneState::default(),
            tag_filter,
            property_entries: Vec::new(),
            property_state: PropertyPaneState::default(),
            property_filter,
            name_finder: None,
            mention_generation: 0,
            status: "Pass a vault folder as the first argument".into(),
            notices: Notices::default(),
            words: 0,
            chars: 0,
            _watcher: None,
            save_task: None,
            started: false,
            _subscriptions: subscriptions,
        };
        if let Some(root) = root {
            ws.load_vault(root, window, cx);
        }
        ws
    }

    pub(crate) fn load_vault(
        &mut self,
        root: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.load_vault_with(root, true, window, cx);
    }

    /// [`Self::load_vault`], saying whether the vault goes on the
    /// switcher's list once it is open.
    fn load_vault_with(
        &mut self,
        root: PathBuf,
        record: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.leave_vault(cx);
        self.record_vault = record;
        self.vault_name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
            .into();
        self.loading = Some(format!("Indexing {}…", root.display()).into());
        self.status = "Indexing vault".into();
        // Restoring the layout may have to make tab groups, and a group's
        // editor is made with the window, so the vault comes back in it.
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { VaultState::load(&root) })
                .await;
            this.update_in(cx, |ws, window, cx| match result {
                Ok(state) => ws.set_state(state, window, cx),
                Err(err) => {
                    ws.loading = None;
                    ws.status = format!("Vault failed to open: {err}").into();
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Puts the vault that is open away before another loads: the note's
    /// edits and the layout reach its disk as they would on quit, every
    /// group but one closes and that one is left with an empty tab, and
    /// what was read from its `workspace.json` is forgotten, so the next
    /// vault restores into a blank main area the way a fresh launch does.
    /// With no vault open there is nothing to do.
    fn leave_vault(&mut self, cx: &mut Context<Self>) {
        if self.state.is_none() {
            return;
        }
        self.on_quit(cx);
        let ids: Vec<u64> = self.groups.iter().map(|g| g.id).collect();
        for id in ids {
            self.close_group(id, false, cx);
        }
        self.closed_tabs.clear();
        self.detached = None;
        self.layout = None;
        self.layout_restored = false;
        self.last_open_files.clear();
        self._watcher = None;
        self.state = None;
    }

    /// Records a property type in the vault's `types.json` and redraws the
    /// views from the new table.
    fn assign_property_type(
        &mut self,
        key: String,
        ty: silica_formats::properties::PropertyType,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state.clone() else {
            return;
        };
        match state.assign_property_type(&key, ty) {
            Ok(types) => {
                self.reading
                    .update(cx, |r, _| r.set_property_types(types.clone()));
                self.editor.update(cx, |e, _| e.set_property_types(types));
                self.refresh_property_rows(&state);
                cx.notify();
            }
            Err(err) => {
                self.report(format!("types.json could not be written: {err}"), cx);
            }
        }
    }

    /// Pushes what `app.json` says into the open views: the parts of the
    /// editor and reading view that a setting changes while the vault is
    /// open. Called when the vault loads and again whenever Settings
    /// writes.
    fn apply_app_config(&mut self, state: &VaultState, cx: &mut Context<Self>) {
        let readable = state.app().readable_line_length.unwrap_or(true);
        let properties =
            PropertiesInDocument::from_config(state.app().properties_in_document.as_deref());
        let (smart, use_tab, tab_size) = (
            state.app().smart_indent_list.unwrap_or(true),
            state.app().use_tab.unwrap_or(true),
            state.app().tab_size.unwrap_or(4),
        );
        let build_options = state.build_options();
        let link_options = silica_suggest::Options {
            format: state.link_format(),
            markdown_links: state.app().use_markdown_links.unwrap_or(false),
        };
        // Every group's views, the active one's among them.
        for group in &self.groups {
            group.reading.update(cx, |r, cx| {
                r.set_readable_line_length(readable);
                r.set_property_types(state.property_types().clone());
                r.set_properties_in_document(properties);
                r.set_fold_headings(state.app().fold_heading.unwrap_or(true));
                cx.notify();
            });
            group.editor.update(cx, |e, cx| {
                e.set_link_options(link_options);
                e.set_readable_line_length(readable);
                e.set_indent_options(smart, use_tab, tab_size);
                e.set_property_types(state.property_types().clone());
                e.set_properties_in_document(properties);
                e.set_build_options(build_options, cx);
            });
        }
    }

    /// Writes a setting the Settings modal changed, then applies it. The
    /// modal redraws from the vault's config either way, so a write that
    /// fails leaves the control as it was and says so in the status bar.
    /// Rebuilds the command registry and every key bound from it: the
    /// commands of a core plugin that is off are not in it, and
    /// `hotkeys.json` is applied over what is left.
    fn rebuild_commands(
        &mut self,
        state: &VaultState,
        overrides: Option<&Hotkeys>,
        cx: &mut Context<Self>,
    ) {
        let mut registry = CommandRegistry::with_known_ids();
        registry.retain_enabled(|plugin| state.core_plugin_enabled(plugin));
        match overrides {
            Some(overrides) => registry.apply_overrides(overrides),
            None => {
                if let Ok(hotkeys) = state.vault().hotkeys() {
                    registry.apply_overrides(&hotkeys);
                }
            }
        }
        let registry = Arc::new(registry);
        crate::bind_all_keys(cx, &registry);
        cx.set_global(Commands(registry));
    }

    fn change_setting(
        &mut self,
        change: SettingChange,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state.clone() else {
            return;
        };
        if let Err(err) = state.apply_setting(&change) {
            self.report(format!("{} could not be written: {err}", change.file()), cx);
            return;
        }
        // The registry is what the palette, the Hotkeys tab and the key
        // bindings read. Both a hotkey change and a core-plugin toggle
        // change what belongs in it -- turning a plugin off takes its
        // commands with it, the way Obsidian does -- so either rebuilds it
        // from the files as written and makes every binding again.
        match &change {
            SettingChange::Hotkeys(overrides) => {
                self.rebuild_commands(&state, Some(overrides), cx);
            }
            SettingChange::CorePlugins(_) => self.rebuild_commands(&state, None, cx),
            _ => {}
        }
        // Recompute the global theme live when appearance changes
        // (dark/light, fonts, accent, CSS theme, CSS snippets).
        // Previously these required a restart.
        if matches!(&change, SettingChange::Appearance(_)) {
            self.recompute_theme(&state, window, cx);
        }
        self.apply_app_config(&state, cx);
        self.refresh_vault_views(cx);
        cx.notify();
    }

    /// Rebuild the global [`Theme`] from the vault's current appearance and
    /// app config, the system's dark/light state, and any CSS theme or
    /// snippets on disk. Called from [`change_setting`] so appearance and
    /// font changes take effect without a restart.
    fn recompute_theme(&self, state: &VaultState, window: &mut Window, cx: &mut Context<Self>) {
        let appearance = state.appearance();
        let dark = match appearance.theme.as_deref() {
            Some("moonstone") => false,
            Some("obsidian") => true,
            _ => matches!(
                window.appearance(),
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            ),
        };
        let css_theme = appearance
            .css_theme
            .as_ref()
            .and_then(|name| load_css_theme(&state.root, name));
        let snippets_names = appearance.enabled_css_snippets.as_deref().unwrap_or(&[]);
        let css_snippets = load_css_snippets(&state.root, snippets_names);
        cx.set_global(Theme::from_appearance(
            &appearance,
            dark,
            css_theme.as_ref(),
            &css_snippets,
        ));
    }

    fn set_state(&mut self, state: VaultState, window: &mut Window, cx: &mut Context<Self>) {
        self.loading = None;
        self.status = format!(
            "{} files, {} notes indexed in {:.0} ms",
            state.vault().files().len(),
            state.cache().len(),
            state.index_ms
        )
        .into();
        // A config that would not parse is being stood in for by its
        // defaults, and Settings will refuse to write it. Say so once, on
        // the way in, rather than leaving a tab that looks merely wrong.
        if !state.unreadable_configs.is_empty() {
            let names = state.unreadable_configs.join(", ");
            let one = state.unreadable_configs.len() == 1;
            let (verb, noun) = if one { ("is", "it") } else { ("are", "them") };
            self.report(
                format!(
                    "{names} {verb} not valid JSON; Silica is using defaults and will not overwrite {noun}"
                ),
                cx,
            );
        }
        let state = Arc::new(state);
        self.state = Some(state.clone());
        if self.record_vault {
            let root = state.root.clone();
            self.remember_vault(&root, cx);
        }
        for group in &self.groups {
            Self::give_editor_the_vault(&group.editor, &state, cx);
            Self::give_reading_the_vault(&group.reading, &state, cx);
        }
        self.apply_app_config(&state, cx);
        self.refresh_vault_views(cx);
        self.start_watching(&state, cx);
        if let Some(current) = self.current.clone() {
            self.open_file(current, None, cx);
        } else {
            self.restore_layout(&state, window, cx);
        }
        cx.notify();
    }

    /// Points an editor at the vault: what its embeds render from and what
    /// its autocompletion offers. Every group's editor gets this, the first
    /// when the vault opens and the rest as they are made.
    fn give_editor_the_vault(editor: &Entity<Editor>, state: &Arc<VaultState>, cx: &mut App) {
        let embed_source: Arc<dyn reading::EmbedSource + Send + Sync> = state.clone();
        let build_options = state.build_options();
        editor.update(cx, |e, cx| {
            e.set_vault(Some(embed_source), build_options, cx);
            e.set_suggest_vault(state.clone());
        });
    }

    fn give_reading_the_vault(reading: &Entity<ReadingView>, state: &Arc<VaultState>, cx: &mut App) {
        let state_clone = state.clone();
        reading.update(cx, |r, _cx| {
            let state = state_clone.clone();
            let resolver: crate::reading::view::PreviewHandler = std::rc::Rc::new(move |target, _window, cx: &mut App| {
                let data = crate::page_preview::resolve_preview(target, None, &state)?;
                Some(cx.new(|_| crate::page_preview::PagePreviewView::new(data)).into())
            });
            r.set_preview_resolver(Some(resolver));
        });
    }

    /// Opens today's daily note, creating it if it is not there, or steps
    /// to the previous or next one that exists.
    ///
    /// The configured folder has to exist already — Obsidian refuses rather
    /// than creating it — while folders the format itself names are made on
    /// the way. See
    /// [[Specification#Observed daily notes behavior, Obsidian 1.14.4]].
    fn open_daily(&mut self, step: Option<bool>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let (format, folder, template) = {
            let daily = state.daily_notes();
            (
                non_empty(daily.format.as_deref())
                    .unwrap_or(config::DAILY_NOTE_DEFAULT_FORMAT)
                    .to_string(),
                daily.folder.clone().unwrap_or_default(),
                daily.template.clone().unwrap_or_default(),
            )
        };
        let today = now();
        if let Some(forward) = step {
            // From the note in hand when it is a daily note, else from today.
            let from = self
                .current
                .as_deref()
                .and_then(|path| state.vault().daily_note_day(&format, &folder, path, &today))
                .unwrap_or(today);
            let next = state
                .vault()
                .step_daily_note(&format, &folder, &from, forward);
            match next {
                Some(path) => {
                    self.open_file(path, None, cx);
                    let mode = self.editing_mode();
                    self.set_mode(mode, window, cx);
                }
                None => self.report(
                    if forward {
                        "No next daily note"
                    } else {
                        "No previous daily note"
                    },
                    cx,
                ),
            }
            return;
        }
        let (path, exists) = match state.vault().daily_note(&format, &folder, &today) {
            Ok(found) => found,
            Err(err) => return self.report(err.to_string(), cx),
        };
        if exists {
            // An existing note is opened and not touched, bytes or times.
            self.open_file(path, None, cx);
            let mode = self.editing_mode();
            self.set_mode(mode, window, cx);
            return;
        }
        let text = self.daily_template_text(&state, &template, &path, cx);
        match state.create_note(&path, &text) {
            Ok(changes) => {
                self.after_changes(changes, cx);
                self.open_file(path.clone(), None, cx);
                let mode = self.editing_mode();
                self.set_mode(mode, window, cx);
                self.report(format!("Created {path}"), cx);
            }
            Err(err) => self.report(err, cx),
        }
    }

    /// The bytes a new daily note starts with: its template expanded, or
    /// nothing when none is set or it cannot be read.
    fn daily_template_text(
        &mut self,
        state: &Arc<VaultState>,
        template: &str,
        path: &str,
        cx: &mut Context<Self>,
    ) -> String {
        if template.is_empty() {
            return String::new();
        }
        let template_path = if template.ends_with(".md") {
            template.to_string()
        } else {
            format!("{template}.md")
        };
        let raw = match std::fs::read_to_string(state.vault().absolute(&template_path)) {
            Ok(text) => text,
            Err(err) => {
                self.report(format!("Could not read {template_path}: {err}"), cx);
                return String::new();
            }
        };
        let text = silica_text::FileText::parse(&raw).text;
        let title = basename(path).to_string();
        let templates = state.templates();
        let context = templates::Context {
            at: now(),
            date_format: non_empty(templates.date_format.as_deref())
                .unwrap_or(templates::DEFAULT_DATE_FORMAT),
            time_format: non_empty(templates.time_format.as_deref())
                .unwrap_or(templates::DEFAULT_TIME_FORMAT),
            title: &title,
        };
        templates::expand(&text, &context)
    }

    /// Obsidian's template chooser: the template folder's notes by
    /// basename, sorted, with its own placeholder. An unset or empty
    /// folder still opens the list, which then says why it is empty.
    fn open_template_chooser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.mode.is_editing() {
            return;
        }
        let Some(state) = self.state.clone() else {
            return;
        };
        let folder = state
            .templates()
            .folder
            .clone()
            .unwrap_or_default()
            .trim_matches('/')
            .to_string();
        let empty = if folder.is_empty() {
            "No template folder is set in Settings".to_string()
        } else {
            format!("No templates in {folder}")
        };
        let mut rows: Vec<ChooserRow> = Vec::new();
        if !folder.is_empty() {
            let prefix = format!("{folder}/");
            let vault = state.vault();
            for file in vault.files() {
                if file.kind != FileKind::Markdown || !file.path.starts_with(&prefix) {
                    continue;
                }
                rows.push(ChooserRow {
                    label: basename(&file.path).to_string(),
                    value: file.path.clone(),
                });
            }
        }
        rows.sort_by(|a, b| a.label.cmp(&b.label));
        self.close_editor_suggest(cx);
        self.chooser_purpose = ChooserPurpose::Template;
        self.modal = Modal::Chooser;
        self.chooser.update(cx, |c, cx| {
            c.open(rows, "Type name of a template...", empty, window, cx)
        });
        cx.notify();
    }

    /// "Add note from vault" and "Add media from vault": the chooser over
    /// the vault's notes, or its images, PDFs, audio and video; the pick
    /// becomes a file card at the middle of the canvas.
    fn open_canvas_file_chooser(
        &mut self,
        kind: crate::canvas::FilePick,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let wanted = |k: FileKind| match kind {
            crate::canvas::FilePick::Note => k == FileKind::Markdown,
            crate::canvas::FilePick::Media => matches!(
                k,
                FileKind::Image | FileKind::Pdf | FileKind::Audio | FileKind::Video
            ),
        };
        let mut rows: Vec<ChooserRow> = state
            .vault()
            .files()
            .iter()
            .filter(|f| wanted(f.kind))
            .map(|f| ChooserRow {
                label: match kind {
                    crate::canvas::FilePick::Note => basename(&f.path).to_string(),
                    crate::canvas::FilePick::Media => f.path.clone(),
                },
                value: f.path.clone(),
            })
            .collect();
        rows.sort_by(|a, b| a.label.cmp(&b.label));
        let (placeholder, empty) = match kind {
            crate::canvas::FilePick::Note => ("Type name of a note...", "No notes in the vault"),
            crate::canvas::FilePick::Media => ("Type name of a file...", "No media in the vault"),
        };
        self.chooser_purpose = ChooserPurpose::CanvasFile;
        self.modal = Modal::Chooser;
        self.chooser
            .update(cx, |c, cx| c.open(rows, placeholder, empty, window, cx));
        cx.notify();
    }

    /// `canvas:jump-to-group`: a chooser over the open canvas's labelled
    /// groups; the pick is framed and selected. Obsidian hides the command
    /// when no canvas is active; here it says so.
    fn jump_to_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.main_view(cx) != MainView::Canvas {
            self.report("Canvas: Jump to group needs an open canvas", cx);
            return;
        }
        let rows: Vec<ChooserRow> = self
            .canvas
            .read(cx)
            .group_targets()
            .into_iter()
            .map(|(label, id)| ChooserRow { label, value: id })
            .collect();
        self.chooser_purpose = ChooserPurpose::CanvasGroup;
        self.modal = Modal::Chooser;
        self.chooser.update(cx, |c, cx| {
            c.open(
                rows,
                "Type name of a group...",
                "No groups in this canvas",
                window,
                cx,
            )
        });
        cx.notify();
    }

    /// `canvas:convert-to-file`: the selected text card's text becomes a
    /// note named after its first line (`Untitled` when blank) beside the
    /// canvas, and the card embeds it. Obsidian asks for the name in a
    /// dialog first; here the first line names it.
    fn convert_card_to_file(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        if self.main_view(cx) != MainView::Canvas {
            self.report("Canvas: Convert to file needs a selected text card", cx);
            return;
        }
        let Some((id, text)) = self.canvas.read(cx).selected_text_card() else {
            self.report("Canvas: Convert to file needs a selected text card", cx);
            return;
        };
        let folder = self
            .current
            .as_deref()
            .and_then(|p| p.rsplit_once('/'))
            .map(|(dir, _)| dir)
            .unwrap_or("");
        let first = text
            .lines()
            .map(|l| l.trim().trim_start_matches('#').trim())
            .find(|l| !l.is_empty())
            .unwrap_or("Untitled");
        let name: String = first
            .chars()
            .filter(|c| {
                !matches!(
                    c,
                    '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '[' | ']' | '#' | '^'
                )
            })
            .take(60)
            .collect::<String>()
            .trim()
            .to_string();
        let name = if name.is_empty() {
            "Untitled".to_string()
        } else {
            name
        };
        let path = state.vault().available_path(folder, &name, "md");
        match state.create_note(&path, &text) {
            Ok(changes) => {
                self.after_changes(changes, cx);
                self.canvas
                    .update(cx, |c, cx| c.convert_to_file(&id, &path, cx));
                self.report(format!("Created {path}"), cx);
            }
            Err(err) => self.report(err, cx),
        }
    }

    /// Opens the chooser and picks `query` out of it, for `--template`.
    pub fn pick_template(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.open_template_chooser(window, cx);
        self.chooser.update(cx, |c, cx| c.pick(query, cx));
    }

    /// Reads the chosen template, expands its placeholders and writes it at
    /// the cursor.
    fn insert_template(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        self.modal = Modal::None;
        let Some(state) = self.state.clone() else {
            return;
        };
        let raw = match std::fs::read_to_string(state.vault().absolute(&path)) {
            Ok(text) => text,
            Err(err) => return self.report(format!("Could not read {path}: {err}"), cx),
        };
        // A template's own byte order mark and line endings are its file's,
        // not the target's: only its text is inserted.
        let text = silica_text::FileText::parse(&raw).text;
        let title = self
            .current
            .as_deref()
            .map(basename)
            .unwrap_or_default()
            .to_string();
        let expanded = {
            let templates = state.templates();
            let cx = templates::Context {
                at: now(),
                date_format: non_empty(templates.date_format.as_deref())
                    .unwrap_or(templates::DEFAULT_DATE_FORMAT),
                time_format: non_empty(templates.time_format.as_deref())
                    .unwrap_or(templates::DEFAULT_TIME_FORMAT),
                title: &title,
            };
            templates::expand(&text, &cx)
        };
        self.editor
            .update(cx, |e, cx| e.insert_at_cursors(&expanded, window, cx));
        window.focus(&self.editor.focus_handle(cx), cx);
        cx.notify();
    }

    /// Writes the current date or time at the cursor, in the format
    /// `templates.json` gives — `{{date}}` and `{{time}}` without a
    /// template around them, which is what Obsidian's two commands are.
    fn insert_stamp(&mut self, date: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.mode.is_editing() {
            return;
        }
        let Some(state) = self.state.clone() else {
            return;
        };
        let templates = state.templates();
        let format = if date {
            non_empty(templates.date_format.as_deref()).unwrap_or(templates::DEFAULT_DATE_FORMAT)
        } else {
            non_empty(templates.time_format.as_deref()).unwrap_or(templates::DEFAULT_TIME_FORMAT)
        };
        let missing = templates::unsupported_tokens(format);
        if !missing.is_empty() {
            self.report(
                format!(
                    "Silica cannot render {} in a date format",
                    missing.join(", ")
                ),
                cx,
            );
            return;
        }
        let text = templates::format(format, &now());
        drop(templates);
        self.editor
            .update(cx, |e, cx| e.insert_at_cursors(&text, window, cx));
    }

    /// Rebuilds the All properties rows from the cache and `types.json`.
    fn refresh_property_rows(&mut self, state: &VaultState) {
        let cache = state.cache();
        let types = state.property_types();
        self.property_entries = silica_cache::all_properties(&cache, &types);
    }

    /// Rebuilds the Tags pane's tree from the cache, in the pane's order.
    fn refresh_tag_tree(&mut self, state: &VaultState) {
        let mut tree = panes::tag_tree(state.cache().tags());
        panes::sort_tags(&mut tree, self.tag_state.sort);
        self.tag_tree = tree;
    }

    /// Rebuilds everything that depends on the file list or the whole cache.
    fn refresh_vault_views(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let show_unsupported = state.app().show_unsupported_files.unwrap_or(false);
        let tree = {
            let vault = state.vault();
            FileTree::new(vault.files(), vault.folders(), show_unsupported)
        };
        self.explorer.update(cx, |e, cx| e.set_tree(tree, cx));
        self.refresh_tag_tree(&state);
        self.refresh_property_rows(&state);
        self.switcher
            .update(cx, |s, cx| s.set_state(state.clone(), cx));
        self.search
            .update(cx, |s, cx| s.set_state(state.clone(), cx));
        self.graph
            .update(cx, |g, cx| g.set_state(state.clone(), cx));
        self.settings
            .update(cx, |s, cx| s.set_state(state.clone(), cx));
        self.name_finder = None;
        self.refresh_bookmarks_pane(cx);
        cx.notify();
    }

    fn start_watching(&mut self, state: &Arc<VaultState>, cx: &mut Context<Self>) {
        let (watcher, rx) = match watch::start(&state.root) {
            Ok(w) => w,
            Err(err) => {
                self.report(format!("File watching unavailable: {err}"), cx);
                return;
            }
        };
        self._watcher = Some(watcher);
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(300))
                .await;
            if this.upgrade().is_none() {
                break;
            }
            let mut paths: Vec<PathBuf> = Vec::new();
            while let Ok(p) = rx.try_recv() {
                paths.push(p);
            }
            if paths.is_empty() {
                continue;
            }
            paths.sort();
            paths.dedup();
            if this
                .update(cx, |ws, cx| ws.files_changed(paths, cx))
                .is_err()
            {
                break;
            }
        })
        .detach();
    }

    fn files_changed(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let changes = cx
                .background_executor()
                .spawn(async move { state.apply(&paths) })
                .await;
            this.update(cx, |ws, cx| ws.after_changes(changes, cx)).ok();
        })
        .detach();
    }

    /// Whether `changes` touched `path` only as a modification whose bytes
    /// the open buffer already holds: Silica's own save, arriving back
    /// through the file watcher.
    ///
    /// A content check, not an mtime stamp, so a genuine external write is
    /// never swallowed — a note saved by another app with different bytes
    /// still reloads the view.
    fn is_own_write(&self, path: &str, changes: &Changes, state: &VaultState, cx: &App) -> bool {
        let modified_only = changes
            .files
            .iter()
            .filter(|c| c.path() == path)
            .all(|c| matches!(c, FileChange::Modified(_)));
        if !modified_only {
            return false;
        }
        let editor = self.editor.read(cx);
        if editor.name.as_ref() != path {
            return false;
        }
        state
            .texts()
            .get(path)
            .is_some_and(|text| editor.text() == text.as_str())
    }

    fn after_changes(&mut self, changes: Changes, cx: &mut Context<Self>) {
        if changes.files.is_empty() {
            return;
        }
        let Some(state) = self.state.clone() else {
            return;
        };
        if changes.structural() {
            self.refresh_vault_views(cx);
        } else {
            self.refresh_tag_tree(&state);
            self.refresh_property_rows(&state);
            self.search.update(cx, |s, cx| s.rerun(cx));
        }
        let n = changes.files.len();
        self.status = if n == 1 {
            format!("{} changed", changes.files[0].path()).into()
        } else {
            format!("{n} files changed").into()
        };
        // A card showing one of these files is rebuilt from it, whichever
        // group's canvas holds it.
        for change in &changes.files {
            let path = change.path().to_string();
            for group in &self.groups {
                group.canvas.update(cx, |c, cx| c.invalidate(&path, cx));
            }
        }
        // A base is a view over the whole vault, so any change may change its
        // rows -- unlike a canvas card, which only its own file touches. The
        // base's own bytes are not re-read here: a change to the `.base`
        // itself arrives through `show_file` and reloads the query.
        for group in &self.groups {
            group
                .bases
                .update(cx, |b, cx| b.invalidate(state.clone(), cx));
        }
        // A changed `.pdf` is forgotten by the cache every drawing shares --
        // the embeds and the canvas cards read it afresh on their next
        // frame -- and every view holding it reloads, keeping its page and
        // zoom. The view never writes, so the change is always somebody
        // else's.
        for change in &changes.files {
            let path = change.path();
            if FileKind::from_path(path) != FileKind::Pdf {
                continue;
            }
            crate::pdf::documents(cx).invalidate(&state.vault().absolute(path), cx);
            for group in &self.groups {
                if group.pdf.read(cx).path() == Some(path) {
                    group.pdf.update(cx, |p, cx| p.reload(cx));
                }
            }
        }
        // Back and forward must never land on a file the vault no longer
        // has, whoever removed it; an atomic external save arrives as a
        // removal too, so only a path that is really gone is forgotten.
        for change in &changes.files {
            if let FileChange::Removed(path) = change {
                if !state.vault().absolute(path).exists() {
                    self.forget_history(path);
                    for tab in self.stashed_tabs_mut() {
                        if tab.path.as_deref() == Some(path.as_str()) {
                            tab.path = None;
                        }
                    }
                }
            }
        }
        if let Some(current) = self.current.clone() {
            let removed = changes
                .files
                .iter()
                .any(|c| matches!(c, FileChange::Removed(p) if *p == current));
            if removed && !state.vault().absolute(&current).exists() {
                // The open note is gone from the vault. The buffer stays on
                // screen because it may hold edits that exist nowhere else,
                // but it is no longer a file: the autosave is disarmed and
                // the panes stop describing it.
                self.detach_current(true, &state, cx);
                self.status =
                    format!("{current} was deleted; this buffer is no longer saved anywhere")
                        .into();
            } else if changes.files.iter().any(|c| c.path() == current) {
                // Silica's own autosave comes back through the watcher a
                // moment later. Rebuilding the view from bytes the buffer
                // already holds would re-enter `show_file`, which reveals
                // the row: the Files list would re-centre and every
                // ancestor folder re-expand every few seconds of typing.
                if self.is_own_write(&current, &changes, &state, cx) {
                    self.refresh_panes(&state, cx);
                } else {
                    self.show_file(&current, &state, cx);
                }
            } else {
                self.refresh_panes(&state, cx);
            }
        } else if let Some(detached) = self.detached.clone() {
            // The file the detached buffer came from is back: a slow external
            // unlink-then-rewrite (git checkout, a sync client) lands as two
            // changes either side of the watcher's debounce. Reattach rather
            // than leave the pane showing a note the vault has again under a
            // banner saying it does not; `show_file` keeps unsaved edits and
            // reports that the file changed underneath them.
            if changes.files.iter().any(|c| c.path() == detached.as_ref())
                && state.vault().absolute(&detached).exists()
            {
                self.show_file(&detached, &state, cx);
            }
        }
        cx.notify();
    }

    /// Called once the startup flags have run, so `wait_loaded` can answer
    /// for a window that is finished opening rather than one still doing it.
    pub fn mark_started(&mut self) {
        self.started = true;
    }

    /// Whether the startup flags have been applied.
    pub fn is_started(&self) -> bool {
        self.started
    }

    pub fn is_loaded(&self) -> bool {
        self.state.is_some()
    }

    /// Selects a right-dock tab by name (`--pane` on the command line).
    pub fn show_right_tab(&mut self, name: &str, cx: &mut Context<Self>) {
        let which = match name.to_lowercase().as_str() {
            "outgoing" => RightTab::Outgoing,
            "outline" => RightTab::Outline,
            "tags" => RightTab::Tags,
            "properties" => RightTab::Properties,
            _ => RightTab::Backlinks,
        };
        self.select_right_tab(which, cx);
    }

    /// Shows a right-dock tab, bringing what it shows up to date first: the
    /// Outline is kept current only while it is the pane on screen.
    fn select_right_tab(&mut self, which: RightTab, cx: &mut Context<Self>) {
        self.right_tab = which;
        self.refresh_outline_from_buffer(cx);
        cx.notify();
    }

    pub fn editor(&self) -> &Entity<Editor> {
        &self.editor
    }

    pub fn mode(&self) -> ViewMode {
        self.mode
    }

    /// The vault path shown in the main area.
    pub fn current(&self) -> Option<&str> {
        self.current.as_deref()
    }

    /// Shows the Search tab with `query`; focuses the field when a window is at hand.
    /// The dock comes back first: a focus handle on a pane that is not painted
    /// is outside the dispatch tree, so it would take every key and handle none.
    pub fn show_search(
        &mut self,
        query: &str,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        self.left_visible = true;
        self.left_tab = LeftTab::Search;
        self.search.update(cx, |s, cx| s.set_query(query, cx));
        if let Some(window) = window {
            window.focus(&self.search.focus_handle(cx), cx);
        }
        cx.notify();
    }

    /// Shows the vault-wide search pane with replace row open.
    #[allow(dead_code)]
    pub fn show_replace(
        &mut self,
        query: &str,
        replace_text: &str,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        self.left_visible = true;
        self.left_tab = LeftTab::Search;
        self.search.update(cx, |s, cx| {
            s.set_query(query, cx);
            s.set_replace_open(true, cx);
            if !replace_text.is_empty() {
                s.set_replace_text(replace_text, cx);
            }
        });
        if let Some(window) = window {
            let handle = self.search.read(cx).replace_focus_handle(cx);
            window.focus(&handle, cx);
        }
        cx.notify();
    }

    /// Opens the quick switcher pre-filled with `query`.
    pub fn open_switcher(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        let mut recent: Vec<String> = Vec::new();
        for entry in self.history.iter().rev() {
            if !recent.contains(&entry.path) {
                recent.push(entry.path.clone());
            }
        }
        self.close_editor_suggest(cx);
        self.modal = Modal::Switcher;
        self.switcher
            .update(cx, |s, cx| s.open(recent, query, window, cx));
        cx.notify();
    }

    /// Opens the command palette pre-filled with `query`.
    pub fn open_palette(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        let registry = cx.global::<Commands>().0.clone();
        let pinned: Vec<String> = self
            .state
            .as_ref()
            .map(|s| s.palette.pinned.clone())
            .unwrap_or_default();
        // The palette is what the check callbacks decide, so the list is
        // built with the keyboard where it is now: opening the palette
        // from a reading leaf does not offer the editing commands.
        let mut commands: Vec<PaletteCommand> = registry
            .iter()
            .filter(|c| self.can_run(&c.id, window, cx))
            .map(|c| PaletteCommand::from_command(c, pinned.contains(&c.id)))
            .collect();
        // The commands the loaded plugins registered, under the names
        // Obsidian shows them by. Loading happens here and in Settings, both
        // on this thread: the runtimes are JavaScript, and this is the only
        // thread they can be made on.
        if let Some(state) = self.state.as_ref() {
            silica_plugins::live::ensure(&state.root, state.vault().config_dir(), &state.plugins);
        }
        silica_plugins::live::hosts(|hosts| {
            for host in hosts {
                for command in &host.registrations().commands {
                    commands.push(PaletteCommand {
                        id: command.id.clone(),
                        name: command.name.clone(),
                        hotkeys: Vec::new(),
                        pinned: pinned.contains(&command.id),
                    });
                }
            }
        });
        silica_plugins::live::native(|native| {
            for plugin in native {
                for command in &plugin.commands() {
                    commands.push(PaletteCommand {
                        id: command.id.clone(),
                        name: command.name.clone(),
                        hotkeys: Vec::new(),
                        pinned: pinned.contains(&command.id),
                    });
                }
            }
        });
        let recent = self.recent_commands.clone();
        self.close_editor_suggest(cx);
        self.modal = Modal::Palette;
        self.palette
            .update(cx, |p, cx| p.open(commands, recent, query, window, cx));
        cx.notify();
    }

    /// Opens a Settings dropdown's options, for `--menu`.
    pub fn open_settings_menu(&mut self, key: &str, cx: &mut Context<Self>) {
        self.settings.update(cx, |s, cx| s.open_menu(key, cx));
    }

    /// Opens the Settings modal, on `tab` when given (`--settings` on the command line).
    pub fn open_settings(
        &mut self,
        tab: Option<settings::Tab>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_editor_suggest(cx);
        self.modal = Modal::Settings;
        self.settings.update(cx, |s, cx| s.open(tab, window, cx));
        cx.notify();
    }

    /// A modal going up takes the autocompletion popup down with it: the
    /// popup would otherwise stay painted under the modal, still holding
    /// the keys it binds.
    fn close_editor_suggest(&mut self, cx: &mut Context<Self>) {
        self.editor.update(cx, |e, cx| e.close_suggest(cx));
    }

    fn close_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal != Modal::None {
            self.modal = Modal::None;
            let mode = self.mode;
            self.set_mode(mode, window, cx);
        }
    }

    fn run_command_action(
        &mut self,
        action: &RunCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = action.id.clone();
        self.run_command(&id, window, cx);
    }

    /// Runs a command by its Obsidian id; unknown or not-yet-implemented
    /// ids are reported in the status bar.
    pub fn run_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        // A command whose core plugin is off is not in the registry, so it
        // is in no palette and on no key. Asked for by id -- over the
        // control socket, or by a binding made before the toggle -- it
        // says so rather than running anyway.
        // A plugin command is not in the registry either: it belongs to the
        // plugin's own runtime, on the thread the app runs on. A plugin that
        // failed to load registered nothing, so its commands are unknown here
        // like any other unknown id.
        let state = self.state.clone();
        let current = self.current.clone();
        let ctx = PluginContext {
            vault_root: state.as_ref().map(|s| s.root.as_path()).unwrap_or(std::path::Path::new("")),
            read_file: Box::new({
                let state = state.clone();
                move |path| {
                    let state = state.clone();
                    let root = state.as_ref().map(|s| s.root.clone()).unwrap_or_default();
                    std::fs::read_to_string(root.join(path)).map_err(|e| e.to_string())
                }
            }),
            write_file: Box::new({
                let state = state.clone();
                move |path, text| {
                    let state = state.clone();
                    let root = state.as_ref().map(|s| s.root.clone()).unwrap_or_default();
                    let full_path = root.join(path);
                    if let Some(parent) = full_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    std::fs::write(&full_path, text).map_err(|e| e.to_string())
                }
            }),
            append_file: Box::new({
                let state = state.clone();
                move |path, text| {
                    let state = state.clone();
                    let root = state.as_ref().map(|s| s.root.clone()).unwrap_or_default();
                    let full_path = root.join(path);
                    let mut existing = std::fs::read_to_string(&full_path).unwrap_or_default();
                    existing.push_str(text);
                    std::fs::write(&full_path, existing).map_err(|e| e.to_string())
                }
            }),
            create_file: Box::new({
                let state = state.clone();
                move |path, text| {
                    let state = state.clone();
                    let root = state.as_ref().map(|s| s.root.clone()).unwrap_or_default();
                    let full_path = root.join(path);
                    if full_path.exists() {
                        return Err(format!("'{path}' already exists"));
                    }
                    if let Some(parent) = full_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    std::fs::write(&full_path, text).map_err(|e| e.to_string())
                }
            }),
            create_folder: Box::new({
                let state = state.clone();
                move |path| {
                    let state = state.clone();
                    let root = state.as_ref().map(|s| s.root.clone()).unwrap_or_default();
                    std::fs::create_dir_all(root.join(path)).map_err(|e| e.to_string())
                }
            }),
            delete_file: Box::new({
                let state = state.clone();
                move |path| {
                    let state = state.clone();
                    let root = state.as_ref().map(|s| s.root.clone()).unwrap_or_default();
                    std::fs::remove_file(root.join(path)).map_err(|e| e.to_string())
                }
            }),
            rename_file: Box::new({
                let state = state.clone();
                move |from, to| {
                    let state = state.clone();
                    let root = state.as_ref().map(|s| s.root.clone()).unwrap_or_default();
                    std::fs::rename(root.join(from), root.join(to)).map_err(|e| e.to_string())
                }
            }),
            list_dir: Box::new({
                let state = state.clone();
                move |path| {
                    let state = state.clone();
                    let root = state.as_ref().map(|s| s.root.clone()).unwrap_or_default();
                    let entries = std::fs::read_dir(root.join(path)).map_err(|e| e.to_string())?;
                    Ok(entries
                        .filter_map(|e| e.ok())
                        .map(|e| e.file_name().to_string_lossy().to_string())
                        .collect())
                }
            }),
            file_exists: Box::new({
                let state = state.clone();
                move |path| {
                    let state = state.clone();
                    let root = state.as_ref().map(|s| s.root.clone()).unwrap_or_default();
                    Ok(root.join(path).exists())
                }
            }),
            metadata_cache: Box::new({
                let state = state.clone();
                move |path| {
                    let state = state.clone();
                    let root = state.as_ref().map(|s| s.root.clone()).unwrap_or_default();
                    let full_path = root.join(path);
                    let text = std::fs::read_to_string(&full_path).map_err(|e| e.to_string())?;
                    let meta = silica_cache::note_properties(&text)
                        .ok_or_else(|| "no metadata".to_string())?;
                    Ok(format!("{}", meta.entries.len()))
                }
            }),
            open_note: Box::new({
                let state = state.clone();
                move |path| {
                    let state = state.clone();
                    let root = state.as_ref().map(|s| s.root.clone()).unwrap_or_default();
                    let full_path = root.join(path);
                    if !full_path.exists() {
                        return Err(format!("'{path}' does not exist"));
                    }
                    // Open the note by setting current and reloading the model
                    // This is handled by the workspace; for now just return OK
                    // and let the caller handle it. The plugin signals success.
                    Ok(())
                }
            }),
            current_note: Box::new(move || current.clone()),
        };

        if let Some((outcome, _notices)) = silica_plugins::live::run(id, &ctx) {
            if let Err(reason) = outcome {
                self.report(reason, cx);
            }
            return;
        }

        if let Some(plugin) = silica_commands::plugin_of(id) {
            let enabled = self
                .state
                .as_ref()
                .is_none_or(|state| state.core_plugin_enabled(plugin));
            if !enabled {
                let name = CORE_PLUGINS
                    .iter()
                    .find(|p| p.id == plugin)
                    .map(|p| p.name)
                    .unwrap_or(plugin);
                self.report(format!("{name} is turned off in Settings"), cx);
                return;
            }
        }
        if RUNNABLE.contains(&id) {
            self.recent_commands.retain(|r| r != id);
            self.recent_commands.insert(0, id.to_string());
            self.recent_commands.truncate(20);
        }
        match id {
            "command-palette:open" => self.open_palette("", window, cx),
            "editor:save-file" => self.flush_save(cx),
            "editor:open-search" => self.open_find("", false, window, cx),
            "editor:open-search-replace" => self.open_find("", true, window, cx),
            "editor:add-cursor-above" | "editor:add-cursor-below" => {
                if self.editing_or_report(id, window, cx) {
                    let above = id == "editor:add-cursor-above";
                    self.editor
                        .update(cx, |e, cx| e.add_cursor(above, window, cx));
                }
            }
            "editor:toggle-source" => {
                let mode = match self.mode {
                    ViewMode::LivePreview => ViewMode::Source,
                    _ => ViewMode::LivePreview,
                };
                self.set_mode(mode, window, cx);
            }
            "switcher:open" => self.open_switcher("", window, cx),
            "global-search:open" => self.open_search_action(&OpenSearch, window, cx),
            "graph:open" => self.open_graph_action(&OpenGraph, window, cx),
            // Asked for by id with no file open -- the palette does not offer
            // it, and `can_run` says so -- this opens nothing, which is what
            // Obsidian's `checkCallback` does rather than falling back to the
            // global graph.
            "graph:open-local" if self.current.is_none() => {}
            "graph:open-local" => self.show_graph(true, window, cx),
            "canvas:new-file" => self.new_canvas("", cx),
            "bases:new-file" => self.new_base("", cx),
            "bases:add-view" => self.add_base_view(cx),
            "bases:insert" => self.insert_base(window, cx),
            "bases:copy-table" => self.copy_base_table(window, cx),
            "bases:change-view" => self.change_base_view(window, cx),
            "bases:add-item" => self.add_base_item(cx),
            "canvas:jump-to-group" => self.jump_to_group(window, cx),
            "canvas:convert-to-file" => self.convert_card_to_file(cx),
            "markdown:toggle-preview" => self.toggle_mode(&ToggleMode, window, cx),
            // The properties block is the same editable widget in reading
            // view, and `editor:toggle-fold-properties` was observed running
            // there with the preview leaf active (Specification.md, "Folding"),
            // so these three ask only that the note hold the keyboard.
            //
            // Obsidian opens an unnamed property row and writes nothing until
            // it is named, which is what `add_property_row` does.
            "markdown:add-metadata-property" => {
                if self.note_has_keys(window, cx) {
                    self.editor
                        .update(cx, |e, cx| e.add_property_row(window, cx));
                }
            }
            "markdown:clear-metadata-properties" => {
                if self.note_has_keys(window, cx) {
                    self.editor
                        .update(cx, |e, cx| e.clear_properties(window, cx));
                }
            }
            "editor:toggle-fold-properties" => {
                if self.note_has_keys(window, cx) {
                    if self.mode.is_editing() {
                        self.editor.update(cx, |e, cx| e.toggle_properties_fold(cx));
                    } else {
                        self.reading
                            .update(cx, |r, cx| r.toggle_properties_fold(cx));
                    }
                }
            }
            // The five fold commands pass their check only with a markdown
            // view focused in an editing mode, as observed; none has a
            // default hotkey.
            "editor:toggle-fold" => {
                if self.editing_or_report(id, window, cx) {
                    self.editor.update(cx, |e, cx| e.toggle_fold(cx));
                }
            }
            "editor:fold-all" => {
                if self.editing_or_report(id, window, cx) {
                    self.editor.update(cx, |e, cx| e.fold_all(cx));
                }
            }
            "editor:unfold-all" => {
                if self.editing_or_report(id, window, cx) {
                    self.editor.update(cx, |e, cx| e.unfold_all(cx));
                }
            }
            "editor:fold-more" | "editor:fold-less" => {
                if self.editing_or_report(id, window, cx) {
                    let more = id == "editor:fold-more";
                    self.editor.update(cx, |e, cx| e.fold_step(more, cx));
                }
            }
            "editor:insert-table" => {
                if self.editing_or_report(id, window, cx) {
                    self.editor.update(cx, |e, cx| e.insert_table(window, cx));
                }
            }
            // The wrapping toggles: each writes the pair Obsidian's
            // markup uses around the selection, or takes it off again.
            "editor:toggle-bold"
            | "editor:toggle-italics"
            | "editor:toggle-strikethrough"
            | "editor:toggle-highlight"
            | "editor:toggle-code"
            | "editor:toggle-inline-math"
            | "editor:toggle-comments" => {
                if self.editing_or_report(id, window, cx) {
                    let marker = match id {
                        "editor:toggle-bold" => "**",
                        "editor:toggle-italics" => "*",
                        "editor:toggle-strikethrough" => "~~",
                        "editor:toggle-highlight" => "==",
                        "editor:toggle-code" => "`",
                        "editor:toggle-inline-math" => "$",
                        _ => "%%",
                    };
                    self.editor
                        .update(cx, |e, cx| e.toggle_wrap(marker, window, cx));
                }
            }
            "editor:toggle-bullet-list"
            | "editor:toggle-numbered-list"
            | "editor:toggle-blockquote" => {
                if self.editing_or_report(id, window, cx) {
                    let kind = match id {
                        "editor:toggle-bullet-list" => LinePrefix::Bullet,
                        "editor:toggle-numbered-list" => LinePrefix::Ordered,
                        _ => LinePrefix::Quote,
                    };
                    self.editor
                        .update(cx, |e, cx| e.toggle_line_prefix(kind, window, cx));
                }
            }
            "editor:toggle-checklist-status" => {
                if self.editing_or_report(id, window, cx) {
                    self.editor
                        .update(cx, |e, cx| e.toggle_checklist(window, cx));
                }
            }
            "editor:cycle-list-checklist" => {
                if self.editing_or_report(id, window, cx) {
                    self.editor
                        .update(cx, |e, cx| e.cycle_checklist(window, cx));
                }
            }
            // The six colours write one emoji after the opening `==`.
            "editor:toggle-highlight-red"
            | "editor:toggle-highlight-orange"
            | "editor:toggle-highlight-yellow"
            | "editor:toggle-highlight-green"
            | "editor:toggle-highlight-blue"
            | "editor:toggle-highlight-purple" => {
                if self.editing_or_report(id, window, cx) {
                    let color = id
                        .rsplit('-')
                        .next()
                        .and_then(silica_markdown::HighlightColor::from_name);
                    self.editor
                        .update(cx, |e, cx| e.toggle_highlight(color, window, cx));
                }
            }
            "editor:clear-formatting" => {
                if self.editing_or_report(id, window, cx) {
                    self.editor
                        .update(cx, |e, cx| e.clear_formatting(window, cx));
                }
            }
            "editor:insert-callout"
            | "editor:insert-codeblock"
            | "editor:insert-mathblock"
            | "editor:insert-horizontal-rule"
            | "editor:insert-tag"
            | "editor:insert-footnote" => {
                if self.editing_or_report(id, window, cx) {
                    if let Some(kind) = BlockInsert::from_id(id) {
                        self.editor
                            .update(cx, |e, cx| e.insert_block(kind, window, cx));
                    }
                }
            }
            // "Toggle heading" is a real palette entry whose check passes
            // and which changes nothing: twelve line shapes, three ways of
            // running it and ten presses in a row all came back
            // byte-identical (Specification.md, "Observed editing command
            // behavior"). Registering the id and doing nothing is parity;
            // it is not a bug here.
            "editor:set-heading" => {
                self.editing_or_report(id, window, cx);
            }
            "editor:set-heading-0"
            | "editor:set-heading-1"
            | "editor:set-heading-2"
            | "editor:set-heading-3"
            | "editor:set-heading-4"
            | "editor:set-heading-5"
            | "editor:set-heading-6" => {
                if self.editing_or_report(id, window, cx) {
                    let level = id
                        .rsplit('-')
                        .next()
                        .and_then(|n| n.parse::<u8>().ok())
                        .unwrap_or(0);
                    self.editor
                        .update(cx, |e, cx| e.set_heading(level, window, cx));
                }
            }
            "editor:indent-list" | "editor:unindent-list" => {
                if self.editing_or_report(id, window, cx) {
                    let indent = id == "editor:indent-list";
                    self.editor
                        .update(cx, |e, cx| e.indent_list(indent, window, cx));
                }
            }
            "editor:delete-paragraph" => {
                if self.editing_or_report(id, window, cx) {
                    self.editor
                        .update(cx, |e, cx| e.delete_paragraph(window, cx));
                }
            }
            "editor:swap-line-up" | "editor:swap-line-down" => {
                if self.editing_or_report(id, window, cx) {
                    let up = id == "editor:swap-line-up";
                    self.editor.update(cx, |e, cx| e.swap_lines(up, window, cx));
                }
            }
            "editor:insert-link" | "editor:insert-wikilink" | "editor:insert-embed" => {
                if self.editing_or_report(id, window, cx) {
                    let kind = match id {
                        "editor:insert-link" => LinkSyntax::Markdown,
                        "editor:insert-wikilink" => LinkSyntax::Wikilink,
                        _ => LinkSyntax::Embed,
                    };
                    self.editor
                        .update(cx, |e, cx| e.insert_link(kind, window, cx));
                }
            }
            "editor:follow-link" => {
                if self.editing_or_report(id, window, cx) {
                    self.editor.update(cx, |e, cx| e.follow_link_at_cursor(cx));
                }
            }
            "editor:open-link-in-new-leaf" => {
                if self.editing_or_report(id, window, cx) {
                    self.link_in_new_tab = true;
                    let found = self.editor.update(cx, |e, cx| e.follow_link_at_cursor(cx));
                    if !found {
                        self.link_in_new_tab = false;
                    }
                }
            }
            "insert-current-date" | "insert-current-time" => {
                if self.editing_or_report(id, window, cx) {
                    let date = id == "insert-current-date";
                    self.insert_stamp(date, window, cx);
                }
            }
            "insert-template" => {
                if self.editing_or_report(id, window, cx) {
                    self.open_template_chooser(window, cx);
                }
            }
            "daily-notes" => self.open_daily(None, window, cx),
            "daily-notes:goto-prev" => self.open_daily(Some(false), window, cx),
            "daily-notes:goto-next" => self.open_daily(Some(true), window, cx),
            _ if TableCommand::from_id(id).is_some() => {
                let cmd = TableCommand::from_id(id).expect("checked above");
                if self.editing_or_report(id, window, cx) {
                    let ran = self
                        .editor
                        .update(cx, |e, cx| e.table_command(cmd, window, cx));
                    if !ran {
                        self.report("The cursor is not in a table", cx);
                    }
                }
            }
            "app:go-back" => self.go_back(&GoBack, window, cx),
            "app:go-forward" => self.go_forward(&GoForward, window, cx),
            "app:open-settings" => self.open_settings(None, window, cx),
            "app:toggle-left-sidebar" => {
                self.left_visible = !self.left_visible;
                if !self.left_visible {
                    self.leave_search_field(window, cx);
                }
                cx.notify();
            }
            "app:toggle-right-sidebar" => {
                self.right_visible = !self.right_visible;
                self.refresh_outline_from_buffer(cx);
                cx.notify();
            }
            "file-explorer:open" => {
                self.leave_search_field(window, cx);
                self.left_visible = true;
                self.left_tab = LeftTab::Files;
                self.explorer.update(cx, |e, cx| e.focus_tree(window, cx));
                cx.notify();
            }
            "file-explorer:reveal-active-file" => {
                self.leave_search_field(window, cx);
                self.left_visible = true;
                self.left_tab = LeftTab::Files;
                match self.current.clone() {
                    Some(current) => self
                        .explorer
                        .update(cx, |e, cx| e.reveal_and_focus(&current, window, cx)),
                    None => self.explorer.update(cx, |e, cx| e.focus_tree(window, cx)),
                }
                cx.notify();
            }
            "file-explorer:new-file"
            | "file-explorer:new-file-in-current-tab"
            | "file-explorer:new-file-in-new-pane" => {
                // Obsidian's "Create new note" goes to a new tab too, unless
                // the active one is empty; "in current tab" replaces the
                // note, and "to the right" would split, which is a new tab
                // in the one strip Silica has.
                let fresh_tab = match id {
                    "file-explorer:new-file-in-current-tab" => false,
                    "file-explorer:new-file-in-new-pane" => true,
                    _ => self.current.is_some(),
                };
                // The folder follows the note that was open, so it is
                // settled before the new tab empties the main area.
                let folder = self
                    .state
                    .as_ref()
                    .map(|s| {
                        s.vault().new_note_folder(
                            s.app().new_file_location.as_deref(),
                            s.app().new_file_folder_path.as_deref(),
                            self.current.as_deref(),
                        )
                    })
                    .unwrap_or_default();
                if fresh_tab {
                    self.new_tab(cx);
                }
                self.new_note(&folder, window, cx);
            }
            "file-explorer:new-folder" => self.start_new_folder("", window, cx),
            "file-explorer:move-file" => {
                if let Some(current) = self.current.clone() {
                    self.start_move(&current, window, cx);
                }
            }
            "file-explorer:duplicate-file" => {
                if let Some(current) = self.current.clone() {
                    self.duplicate(&current, cx);
                }
            }
            "workspace:edit-file-title" => {
                if let Some(current) = self.current.clone() {
                    self.start_rename(&current, window, cx);
                }
            }
            "workspace:new-tab" => self.new_tab(cx),
            "workspace:close" => self.close_tab(self.active_tab, cx),
            "workspace:close-others" => self.close_other_tabs(cx),
            "workspace:split-vertical" => self.split(SplitDirection::Vertical, window, cx),
            "workspace:split-horizontal" => self.split(SplitDirection::Horizontal, window, cx),
            "workspace:close-tab-group" => {
                let id = self.active_group_id();
                self.close_group(id, true, cx);
            }
            // Obsidian's "Close others in tab group" closes the group's
            // other tabs, which is what `close-others` does too.
            "workspace:close-others-tab-group" => self.close_other_tabs(cx),
            "editor:focus-left" => self.focus_neighbor(SplitDirection::Vertical, false, cx),
            "editor:focus-right" => self.focus_neighbor(SplitDirection::Vertical, true, cx),
            "editor:focus-top" => self.focus_neighbor(SplitDirection::Horizontal, false, cx),
            "editor:focus-bottom" => self.focus_neighbor(SplitDirection::Horizontal, true, cx),
            // "Open link under cursor to the right": a split right, then the
            // link followed in the new group.
            "editor:open-link-in-new-split" => {
                if self.editing_or_report(id, window, cx) {
                    if let Some(target) = self.editor.read(cx).link_target_at_cursor() {
                        self.split(SplitDirection::Vertical, window, cx);
                        self.follow_link(target, cx);
                    }
                }
            }
            "workspace:next-tab" => self.cycle_tab(1, cx),
            "workspace:previous-tab" => self.cycle_tab(self.tabs.len().saturating_sub(1), cx),
            "workspace:goto-last-tab" => self.activate_tab(self.tabs.len().saturating_sub(1), cx),
            "workspace:undo-close-pane" => self.undo_close_tab(cx),
            _ if id.starts_with("workspace:goto-tab-") => {
                let n: usize = id
                    .rsplit('-')
                    .next()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(0);
                if n >= 1 && n <= self.tabs.len() {
                    self.activate_tab(n - 1, cx);
                }
            }
            "app:delete-file" => {
                if let Some(current) = self.current.clone() {
                    self.start_delete(&current, window, cx);
                }
            }
            "backlink:open" | "backlink:open-backlinks" => {
                self.right_visible = true;
                self.show_right_tab("backlinks", cx);
            }
            "outgoing-links:open" | "outgoing-links:open-for-current" => {
                self.right_visible = true;
                self.show_right_tab("outgoing", cx);
            }
            "outline:open" | "outline:open-for-current" => {
                self.right_visible = true;
                self.show_right_tab("outline", cx);
            }
            "bookmarks:open" => {
                self.left_visible = true;
                self.left_tab = LeftTab::Bookmarks;
                cx.notify();
            }
            "bookmarks:bookmark-current-view" => self.start_bookmark_current_view(window, cx),
            "bookmarks:bookmark-current-search" => {
                let query = self.search.read(cx).query(cx);
                if query.trim().is_empty() {
                    self.report("No search to bookmark: the Search pane has no query", cx);
                } else {
                    self.start_bookmark(BookmarkDraft::Search(query), window, cx);
                }
            }
            "bookmarks:unbookmark-current-view" => {
                let Some(current) = self.current.clone() else {
                    self.report("No file is open", cx);
                    return;
                };
                let had = self
                    .state
                    .as_ref()
                    .is_some_and(|s| s.bookmarks().has_file(&current));
                if had {
                    self.change_bookmarks(cx, move |b| {
                        b.remove_last_file(&current);
                    });
                } else {
                    self.report(format!("{current} is not bookmarked"), cx);
                }
            }
            "bookmarks:bookmark-current-heading" => {
                if self.editing_or_report(id, window, cx) {
                    match (
                        self.current.clone(),
                        self.editor.read(cx).heading_subpath_at_cursor(),
                    ) {
                        (Some(path), Some(subpath)) => self.start_bookmark(
                            BookmarkDraft::File {
                                path,
                                subpath: Some(subpath),
                            },
                            window,
                            cx,
                        ),
                        _ => self.report("The cursor is not on a heading", cx),
                    }
                }
            }
            "bookmarks:bookmark-current-section" => {
                if self.editing_or_report(id, window, cx) {
                    let Some(path) = self.current.clone() else {
                        return;
                    };
                    if let Some(block_id) = self
                        .editor
                        .update(cx, |e, cx| e.block_id_at_cursor(window, cx))
                    {
                        self.start_bookmark(
                            BookmarkDraft::File {
                                path,
                                subpath: Some(format!("#^{block_id}")),
                            },
                            window,
                            cx,
                        );
                    }
                }
            }
            "bookmarks:bookmark-all-tabs" => {
                let mut order = Vec::new();
                self.layout_tree.groups(&mut order);
                let mut paths = Vec::new();
                for id in order {
                    if let Some(ix) = self.group_ix(id) {
                        let tabs = if ix == self.active_group {
                            self.tab_snapshots()
                        } else {
                            self.groups[ix].tabs.clone()
                        };
                        paths.extend(tabs.into_iter().filter_map(|t| t.path));
                    }
                }
                if paths.is_empty() {
                    self.report("No open notes to bookmark", cx);
                } else {
                    self.start_bookmark(BookmarkDraft::Tabs(paths), window, cx);
                }
            }
            "tag-pane:open" => {
                self.right_visible = true;
                self.show_right_tab("tags", cx);
            }
            "properties:open" => {
                self.right_visible = true;
                self.show_right_tab("properties", cx);
            }
            "app:open-help" => self.open_help(window, cx),
            // "Manage vaults" raises the vault manager; Obsidian's does
            // the same (its `app:open-vault` left no menu and no modal
            // behind, and raised `starter.html` — probe of 2026-10-29).
            "app:open-vault" => self.open_vault_manager(window, cx),
            "app:switch-vault" => self.toggle_vault_menu(cx),
            "app:open-another-vault" => self.open_another_vault(window, cx),
            "app:open-sandbox-vault" => self.open_sandbox_vault(window, cx),
            _ => {
                let name = self.command_name(id, cx);
                self.report(format!("{name}: not available in Silica yet"), cx);
            }
        }
    }

    /// "Change vault..." and the footer's name: the switcher's menu, up
    /// from the sidebar's footer, listing every vault this machine has
    /// opened. Obsidian's own `app:switch-vault` raised neither a menu nor
    /// a window in the probe of 2026-10-29; Silica gives it the menu the
    /// footer opens.
    fn toggle_vault_menu(&mut self, cx: &mut Context<Self>) {
        self.vault_menu_open = !self.vault_menu_open;
        cx.notify();
    }

    /// "Manage vaults" and the menu's "Manage vaults...": the vault
    /// manager window, brought forward if it is already up.
    fn open_vault_manager(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.vault_menu_open = false;
        if let Some(handle) = self.vault_window {
            let up = handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok();
            if up {
                cx.notify();
                return;
            }
        }
        let main = window.window_handle();
        match VaultManager::open(cx.weak_entity(), main, cx) {
            Ok(handle) => self.vault_window = Some(handle),
            Err(err) => self.report(format!("Could not open the vault manager: {err}"), cx),
        }
        cx.notify();
    }

    /// "Open help": the Help window, brought forward if it is already up.
    fn open_help(&mut self, window: &Window, cx: &mut Context<Self>) {
        if let Some(handle) = self.help_window {
            let up = handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok();
            if up {
                return;
            }
        }
        let main = window.window_handle();
        match HelpWindow::open(cx.weak_entity(), main, cx) {
            Ok(handle) => self.help_window = Some(handle),
            Err(err) => self.report(format!("Could not open the Help window: {err}"), cx),
        }
    }

    /// "Open sandbox vault": a fresh folder under the temporary directory
    /// with a note or two in it, loaded as the vault in this window and
    /// left off the switcher's list. Obsidian's sandbox is its help vault
    /// copied somewhere disposable; Silica's starts nearly empty.
    pub(crate) fn open_sandbox_vault(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.vault_menu_open = false;
        let root = std::env::temp_dir().join(format!("Silica Sandbox {}", now_millis()));
        if let Err(err) = write_sandbox(&root) {
            self.report(format!("Could not make the sandbox vault: {err}"), cx);
            return;
        }
        self.load_vault_with(root, false, window, cx);
        // Opened once the vault is in, the way `--note` is.
        self.current = Some("Welcome.md".into());
    }

    /// Puts `root` at the head of the switcher's list. The list lives
    /// outside every vault; a machine with nowhere to keep it has no list.
    fn remember_vault(&mut self, root: &Path, cx: &mut Context<Self>) {
        let outcome = (|| -> std::io::Result<()> {
            let Some(mut registry) = VaultRegistry::open()? else {
                return Ok(());
            };
            registry.record_open(root, now_millis());
            registry.save()
        })();
        if let Err(err) = outcome {
            self.report(
                format!("Could not record the vault for the switcher: {err}"),
                cx,
            );
        }
    }

    /// "Open vault...": asks the desktop for a folder and opens it as the
    /// vault in this window. Cancelling the dialog changes nothing.
    fn open_another_vault(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.vault_menu_open = false;
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
                    this.update(cx, |ws, cx| {
                        ws.report(format!("Could not open the folder dialog: {err}"), cx)
                    })
                    .ok();
                    return;
                }
                Err(_) => return,
            };
            this.update_in(cx, |ws, window, cx| ws.load_vault(root, window, cx))
                .ok();
        })
        .detach();
    }

    /// The command's name as the palette shows it, for a message about it.
    fn command_name(&self, id: &str, cx: &App) -> String {
        cx.global::<Commands>()
            .0
            .get(id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| id.to_string())
    }

    /// True when the note itself holds the keyboard: no modal is up and the
    /// focus is on the pane that shows it. A hotkey is bound in the
    /// workspace's key context and so fires wherever the keyboard is, while
    /// Obsidian scopes an editor command to the active editor; without this
    /// a Ctrl+B typed into the search field writes bold markers into the
    /// note behind it.
    /// Whether a command can run right now, which is what the palette
    /// lists and what the socket reports.
    ///
    /// Obsidian hides a command whose check callback fails rather than
    /// offering it and refusing: an `editorCallback` command is not in the
    /// palette of a reading leaf, and a table command is not in it unless
    /// the cursor is in a table. The three readers of this -- the palette
    /// filter, the socket's `runnable` flag and the refusal in
    /// `run_command` -- ask the same question, so they cannot disagree
    /// about what is offered and what happens when it is chosen.
    pub fn can_run(&self, id: &str, window: &Window, cx: &App) -> bool {
        if !RUNNABLE.contains(&id) {
            return false;
        }
        // A command whose core plugin is off is gone from the registry, so
        // this only matters for an id asked for directly.
        if let Some(plugin) = silica_commands::plugin_of(id) {
            if !self
                .state
                .as_ref()
                .is_none_or(|state| state.core_plugin_enabled(plugin))
            {
                return false;
            }
        }
        if TableCommand::from_id(id).is_some() {
            return self.mode.is_editing() && self.editor.read(cx).in_table();
        }
        // `graph:open-local` is a checkCallback: with every markdown leaf
        // detached Obsidian does not list the id at all, rather than offer a
        // command that would open nothing (see [[Specification#`graph:open-
        // local` is not offered with no file open]]). Only the no-file case
        // was observed, so that is all this refuses.
        if id == "graph:open-local" {
            return self.current.is_some();
        }
        if EDITOR_ONLY.contains(&id) {
            return self.mode.is_editing() && self.note_has_keys(window, cx);
        }
        // A base's own commands need one on screen. `bases:copy-table` is
        // offered while a base leaf is active even when the keyboard is
        // elsewhere -- that is the case the probe ran, and it is the one that
        // answers "Unable to copy to your clipboard" -- so this asks only
        // that the base be showing.
        if BASE_ONLY.contains(&id) {
            return self.main_view(cx) == MainView::Bases;
        }
        true
    }

    fn note_has_keys(&self, window: &Window, cx: &App) -> bool {
        self.modal == Modal::None
            && (self.editor.read(cx).focus_handle_ref().is_focused(window)
                || self.reading.focus_handle(cx).is_focused(window))
    }

    /// True when an editor command can run. Obsidian hides a command whose
    /// check callback fails; Silica's palette cannot tell yet, so the
    /// command says why it did nothing rather than silently doing nothing —
    /// but only when the note is what the keys were meant for. A command
    /// that arrives while a dock field or a modal has the keyboard is
    /// addressed to that field, and saying anything about the note would be
    /// an answer to a question nobody asked.
    fn editing_or_report(&mut self, id: &str, window: &Window, cx: &mut Context<Self>) -> bool {
        if !self.note_has_keys(window, cx) {
            return false;
        }
        if self.mode.is_editing() {
            return true;
        }
        let name = self.command_name(id, cx);
        self.report(format!("{name}: only available while editing"), cx);
        false
    }

    fn open_search_action(&mut self, _: &OpenSearch, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.search.read(cx).query(cx);
        self.show_search(&query, Some(window), cx);
    }

    /// Shows the graph (`graph:open`); a second Ctrl+G returns to the note.
    pub fn show_graph(&mut self, local: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.graph.update(cx, |g, cx| g.set_local(local, cx));
        self.set_mode(ViewMode::Graph, window, cx);
    }

    fn open_graph_action(&mut self, _: &OpenGraph, window: &mut Window, cx: &mut Context<Self>) {
        // "Open graph view" toggles the *global* graph. Asked for it with a
        // local graph on screen, Obsidian shows the global one rather than
        // leaving graph mode: `graph:open-local` is not a toggle either, so
        // treating any graph as the one to close made the pair inconsistent.
        if self.mode == ViewMode::Graph && !self.graph.read(cx).is_local() {
            let back = self.mode_before_graph;
            self.set_mode(back, window, cx);
        } else {
            self.show_graph(false, window, cx);
        }
    }

    pub fn set_mode(&mut self, mode: ViewMode, window: &mut Window, cx: &mut Context<Self>) {
        if mode == ViewMode::Graph && self.mode != ViewMode::Graph {
            self.mode_before_graph = self.mode;
        }
        // The history entry for the note on screen tracks the state it is
        // in, so a reader who presses Ctrl+E and then navigates away comes
        // back to the mode they were reading in.
        self.record_history_state(cx);
        if !mode.is_editing() && self.mode.is_editing() {
            // The reading view shows what the editor has: save and rebuild now
            // rather than waiting for the autosave and the watcher.
            if self.editor.read(cx).is_dirty() {
                self.flush_save(cx);
                if let (Some(state), Some(current)) = (self.state.clone(), self.current.clone()) {
                    self.show_file(&current, &state, cx);
                }
            }
        }
        // The fold is shared: the editor's fold collapses the same section
        // in reading view, and a section collapsed there is folded on the
        // way back.
        if mode.is_editing() != self.mode.is_editing() {
            self.sync_folds(mode.is_editing(), cx);
        }
        self.mode = mode;
        if mode.is_editing() {
            self.editor.update(cx, |e, cx| {
                e.set_live_preview(mode == ViewMode::LivePreview, cx)
            });
        }
        self.focus_main(window, cx);
        cx.notify();
    }

    /// Hands the folds across: to the editor when `to_editor`, else to the
    /// reading view, the properties fold with them.
    fn sync_folds(&mut self, to_editor: bool, cx: &mut Context<Self>) {
        if to_editor {
            let (keys, props) = {
                let r = self.reading.read(cx);
                (r.fold_keys(), r.properties_folded())
            };
            self.editor.update(cx, |e, cx| {
                e.set_fold_keys(keys, cx);
                e.set_properties_folded(props, cx);
            });
        } else {
            let (keys, props) = {
                let e = self.editor.read(cx);
                (e.fold_keys(), e.properties_folded())
            };
            self.reading.update(cx, |r, cx| {
                r.set_fold_keys(keys, cx);
                r.set_properties_folded(props, cx);
            });
        }
    }

    /// The fold store of the vault at `root`, opened on first use. The
    /// root is passed in because the first note opens while `self.state`
    /// is still being installed.
    fn fold_store(&mut self, root: &Path) -> Option<&mut fold::FoldStore> {
        if self.fold_store.as_ref().is_none_or(|(r, _)| r != root) {
            self.fold_store = Some((root.to_path_buf(), fold::FoldStore::open(root)));
        }
        self.fold_store.as_mut().map(|(_, s)| s)
    }

    /// Writes the open note's folds to the store: on leaving it, on quit.
    /// The reading view's are handed to the editor first, which is what
    /// knows the extents.
    fn save_folds(&mut self, path: &str, cx: &mut Context<Self>) {
        if FileKind::from_path(path) != FileKind::Markdown {
            return;
        }
        if self.editor.read(cx).name.as_ref() != path {
            return;
        }
        if !self.mode.is_editing() {
            self.sync_folds(true, cx);
        }
        let Some(record) = self.editor.read(cx).fold_record() else {
            return;
        };
        let Some(root) = self.state.as_ref().map(|s| s.root.clone()) else {
            return;
        };
        if let Some(store) = self.fold_store(&root) {
            store.put(path, record);
        }
    }

    /// Restores a note's folds from the store as it opens: the editor takes
    /// the `from` lines, and the reading view the keys they come to.
    fn restore_folds(
        &mut self,
        path: &str,
        text: &str,
        state: &Arc<VaultState>,
        cx: &mut Context<Self>,
    ) {
        let Some(record) = self
            .fold_store(&state.root)
            .and_then(|s| s.get(path).cloned())
        else {
            return;
        };
        let (headings, lists) = {
            let app = state.app();
            (
                app.fold_heading.unwrap_or(true),
                app.fold_indent.unwrap_or(true),
            )
        };
        let doc = silica_markdown::parse(text);
        let foldables = fold::foldables(&doc, text, fold::FoldOptions { headings, lists });
        let froms = record
            .folds
            .iter()
            .filter(|r| **r != fold::PROPERTIES_RANGE)
            .map(|r| r.from);
        let keys = fold::Folds::from_record(&foldables, froms).keys();
        let props = record.folds.contains(&fold::PROPERTIES_RANGE);
        self.editor
            .update(cx, |e, cx| e.set_fold_record(&record, cx));
        self.reading.update(cx, |r, cx| {
            r.set_fold_keys(keys, cx);
            r.set_properties_folded(props, cx);
        });
    }

    /// Asks for the keyboard to be put back on the main area at the next
    /// paint.
    ///
    /// Where focus belongs when the main area changes is one rule — on
    /// whatever that area paints — but most of what changes it cannot
    /// enforce the rule itself: a dock row's click and the socket's `open`
    /// reach the workspace through a subscription or a handler that has no
    /// `Window`. They record the request instead, and [`Workspace::render`],
    /// which has the window and is building the very tree the focus has to
    /// land in, carries it out.
    fn request_focus_main(&mut self, cx: &mut Context<Self>) {
        self.refocus_main = true;
        cx.notify();
    }

    /// Whether the keyboard is somewhere nothing can use it: on the
    /// workspace root, which is where a click in a dock leaves it, on no
    /// element at all, or on a main view the main area is not painting.
    ///
    /// Anything else holds it on purpose — the find panel, the search box,
    /// a property row, a modal's input — and a note opening behind one of
    /// those must not take it away.
    fn keyboard_is_parked(&self, window: &Window, cx: &App) -> bool {
        if window.focused(cx).is_none() || self.focus_handle.is_focused(window) {
            return true;
        }
        let painted = self.main_view(cx);
        let unpainted =
            |view: MainView, handle: &FocusHandle| painted != view && handle.is_focused(window);
        if unpainted(MainView::Editor, self.editor.read(cx).focus_handle_ref())
            || unpainted(MainView::Reading, &self.reading.focus_handle(cx))
            || unpainted(MainView::Graph, &self.graph.focus_handle(cx))
            || unpainted(MainView::Canvas, &self.canvas.focus_handle(cx))
            || unpainted(MainView::Bases, &self.bases.focus_handle(cx))
            || unpainted(MainView::Pdf, &self.pdf.focus_handle(cx))
        {
            return true;
        }
        // A view of a group that is no longer the active one is painted, but
        // the keyboard belongs with the active group's note: a split, a
        // focus command or a close moved the active group and the view it
        // left behind still holds the focus.
        self.groups.iter().enumerate().any(|(ix, g)| {
            ix != self.active_group
                && (g.editor.read(cx).focus_handle_ref().is_focused(window)
                    || g.reading.focus_handle(cx).is_focused(window)
                    || g.canvas.focus_handle(cx).is_focused(window)
                    || g.bases.focus_handle(cx).is_focused(window)
                    || g.pdf.focus_handle(cx).is_focused(window))
        })
    }

    /// Puts the keyboard on whatever the main area actually paints: a handle
    /// on a view that is not there is outside the dispatch tree, and the keys
    /// meant for it reach nothing.
    fn focus_main(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refocus_main = false;
        match self.main_view(cx) {
            MainView::Reading => window.focus(&self.reading.focus_handle(cx), cx),
            MainView::Editor => window.focus(&self.editor.focus_handle(cx), cx),
            MainView::Graph => window.focus(&self.graph.focus_handle(cx), cx),
            MainView::Canvas => window.focus(&self.canvas.focus_handle(cx), cx),
            MainView::Bases => window.focus(&self.bases.focus_handle(cx), cx),
            MainView::Pdf => window.focus(&self.pdf.focus_handle(cx), cx),
            MainView::Empty => window.focus(&self.focus_handle, cx),
        }
    }

    /// Hands the keyboard back to the main area before the left dock
    /// stops painting the search field. Hiding the dock or switching it to
    /// the Files tab unmounts that input, and focus left on an unmounted
    /// element is outside the dispatch tree: every key in the app, the
    /// hotkey that would undo this included, then reaches nothing.
    fn leave_search_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.focus_handle(cx).is_focused(window) {
            self.focus_main(window, cx);
        }
    }

    /// Opens the editor's find panel (with the replace row when asked),
    /// switching to editing first since the panel lives in the editor.
    /// An empty `query` keeps the panel's own (or the selection).
    pub fn open_find(
        &mut self,
        query: &str,
        replace: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.mode.is_editing() {
            let mode = self.editing_mode();
            self.set_mode(mode, window, cx);
        }
        self.editor
            .update(cx, |e, cx| e.open_find(query, replace, window, cx));
    }

    /// The editing mode the vault asks for: Live Preview unless
    /// `livePreview` is off.
    pub fn editing_mode(&self) -> ViewMode {
        let live = self
            .state
            .as_ref()
            .and_then(|s| s.app().live_preview)
            .unwrap_or(true);
        if live {
            ViewMode::LivePreview
        } else {
            ViewMode::Source
        }
    }

    /// The mode a note opens in at startup: `defaultViewMode` (`preview`
    /// is reading view; anything else, Obsidian's default included, edits).
    pub fn default_mode(&self) -> ViewMode {
        let preview = self
            .state
            .as_ref()
            .map(|s| s.app().default_view_mode.as_deref() == Some("preview"))
            .unwrap_or(false);
        if preview {
            ViewMode::Reading
        } else {
            self.editing_mode()
        }
    }

    /// Opens a vault path by exact match or by suffix (`--note` on the command
    /// line); `Name#Heading` or `Name#^id` also scrolls to that section.
    pub fn open_by_name(&mut self, name: &str, cx: &mut Context<Self>) -> bool {
        let Some(state) = self.state.clone() else {
            return false;
        };
        let link = silica_markdown::parse_linktext(name);
        let wanted = link.path;
        let found = state
            .vault()
            .files()
            .iter()
            .map(|f| f.path.as_str())
            .find(|p| {
                *p == wanted
                    || p.ends_with(wanted)
                    || p.trim_end_matches(".md") == wanted
                    || p.trim_end_matches(".md").ends_with(&format!("/{wanted}"))
            })
            .map(str::to_string);
        match found {
            Some(path) => {
                self.open_file(path, link.subpath, cx);
                true
            }
            None => false,
        }
    }

    /// Opens the first markdown note, if any.
    pub fn open_first(&mut self, cx: &mut Context<Self>) {
        let first = self
            .state
            .as_ref()
            .and_then(|s| s.vault().markdown_files().next().map(|f| f.path.clone()));
        if let Some(path) = first {
            self.open_file(path, None, cx);
        }
    }

    /// Opens a vault-relative path in the main area and records it in history.
    pub fn open_file(
        &mut self,
        path: String,
        subpath: Option<Subpath<'_>>,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state.clone() else {
            self.current = Some(path);
            return;
        };
        // The graph is a toggle, not a place a note can open in: a note
        // asked for from anywhere shows the note.
        if self.mode == ViewMode::Graph {
            self.mode = self.mode_before_graph;
        }
        if self.current.as_deref() != Some(path.as_str()) {
            self.flush_save(cx);
            // The entry being left keeps the state it is being left in, so
            // coming back to it comes back to where the reader was.
            self.record_history_state(cx);
            self.history.truncate(self.history_ix);
            self.history
                .push(HistoryEntry::new(path.clone(), self.mode));
            self.history_ix = self.history.len();
            self.last_open_files.retain(|p| *p != path);
            self.last_open_files.insert(0, path.clone());
            self.last_open_files.truncate(LAST_OPEN_FILES_KEPT);
        }
        self.show_file(&path, &state, cx);
        // Opening a note is a move into the main area, wherever it was asked
        // for: an explorer row, a search hit, the switcher, a link, the
        // socket. Obsidian hands the keyboard to the note it just opened, and
        // without this the click that opened it leaves focus on the
        // workspace root, where typing and the arrow keys reach nothing.
        self.request_focus_main(cx);
        if self.current.as_deref() != Some(path.as_str()) {
            // It did not open, so it is not somewhere to go back to either.
            self.forget_history(&path);
            return;
        }
        if let Some(sub) = subpath {
            self.scroll_to_subpath(&path, &sub, &state, cx);
        }
    }

    fn show_file(&mut self, path: &str, state: &Arc<VaultState>, cx: &mut Context<Self>) {
        let previous = self.current.replace(path.to_string());
        if let Some(prev) = previous.clone() {
            if prev != path {
                self.save_folds(&prev, cx);
            }
        }
        if let Some(gone) = self.detached.take() {
            // The buffer was kept because its edits exist nowhere else. There
            // is no file to save them to, and resurrecting one the user
            // deleted would be worse, so the least this can do on the way out
            // is name what it is dropping rather than drop it in silence.
            let lost = {
                let e = self.editor.read(cx);
                e.is_dirty() && e.name.as_ref() != path
            };
            if lost {
                self.report(
                    format!(
                        "Discarded the unsaved edits in {gone}; nothing was saving that buffer"
                    ),
                    cx,
                );
            }
        }
        self.explorer.update(cx, |e, cx| e.follow(path, cx));
        self.graph
            .update(cx, |g, cx| g.set_current(Some(path.to_string()), cx));
        let kind = FileKind::from_path(path);
        if kind != FileKind::Canvas {
            self.release_canvas(cx);
        }
        if kind != FileKind::Base {
            self.release_bases(cx);
        }
        if kind != FileKind::Pdf {
            self.release_pdf(cx);
        }
        match kind {
            FileKind::Canvas => match std::fs::read_to_string(state.vault().absolute(path)) {
                Ok(text) => {
                    // Nothing of a note may show through: the reading view
                    // is emptied and the editor let go of, as for an image.
                    self.reading.update(cx, |r, cx| {
                        r.set_note(
                            None,
                            reading::build_with("", "", state.as_ref(), state.build_options()),
                            cx,
                        )
                    });
                    self.release_buffer(cx);
                    // The same rule as the editor's: the view keeps its
                    // scene when the file is what it last read or wrote
                    // (its own save coming back through the watcher), keeps
                    // unsaved edits when the file changed underneath them,
                    // and reloads otherwise.
                    let (same, matches, dirty) = {
                        let c = self.canvas.read(cx);
                        (c.path() == Some(path), c.matches_file(&text), c.is_dirty())
                    };
                    if same && matches {
                        // Own write or an unrelated event.
                    } else if same && dirty {
                        self.report(format!("{path} changed on disk; unsaved edits kept"), cx);
                    } else if same {
                        self.canvas.update(cx, |c, cx| c.reload(&text, cx));
                    } else {
                        // Where the tab last looked, when it was restored
                        // from the layout for this very file.
                        let view_state = self
                            .tabs
                            .get(self.active_tab)
                            .filter(|t| t.path.as_deref() == Some(path))
                            .and_then(|t| CanvasView::viewport_from_json(t.view_state.as_ref()));
                        let state = state.clone();
                        self.canvas
                            .update(cx, |c, cx| c.load(path, &text, view_state, state, cx));
                    }
                    self.status = path.to_string().into();
                }
                Err(err) => self.unreadable(path, previous, &err, state, cx),
            },
            // A base is drawn by its own view, the way a canvas is. It is
            // read-only in this cut -- Obsidian writes a base only on an
            // edit, or on the close that follows one (see
            // [[Specification#Observed Bases write behavior, Obsidian 1.14.5
            // (installer build 1.13.7)]]) -- so there are no unsaved edits
            // to keep, and the canvas arm's three cases collapse to two:
            // the bytes the view already read, and somebody else's write.
            FileKind::Base => match std::fs::read_to_string(state.vault().absolute(path)) {
                Ok(text) => {
                    // Nothing of a note may show through: the reading view
                    // is emptied and the editor let go of, as for a canvas.
                    self.reading.update(cx, |r, cx| {
                        r.set_note(
                            None,
                            reading::build_with("", "", state.as_ref(), state.build_options()),
                            cx,
                        )
                    });
                    self.release_buffer(cx);
                    let same = {
                        let b = self.bases.read(cx);
                        (b.path() == Some(path)).then(|| b.matches_file(&text))
                    };
                    match same {
                        // Its own read coming back through the watcher, or
                        // an unrelated event: the view has these bytes.
                        Some(true) => {}
                        Some(false) => self.bases.update(cx, |b, cx| b.reload(&text, cx)),
                        None => {
                            let state = state.clone();
                            self.bases
                                .update(cx, |b, cx| b.load(path, &text, state, cx));
                        }
                    }
                    self.status = path.to_string().into();
                }
                Err(err) => self.unreadable(path, previous, &err, state, cx),
            },
            FileKind::Markdown => match std::fs::read_to_string(state.vault().absolute(path)) {
                Ok(text) => {
                    let started = Instant::now();
                    let model =
                        reading::build_with(path, &text, state.as_ref(), state.build_options());
                    let build_ms = started.elapsed().as_secs_f64() * 1000.0;
                    self.words = text.split_whitespace().count();
                    self.chars = text.chars().count();
                    self.reading
                        .update(cx, |r, cx| r.set_note(Some(path.to_string()), model, cx));
                    // The editor keeps its buffer when the file on disk is
                    // what it last saved (or still holds unsaved edits), and
                    // keeps its place when the note changed underneath it.
                    let (same, matches, dirty) = {
                        let e = self.editor.read(cx);
                        (e.name.as_ref() == path, e.matches_file(&text), e.is_dirty())
                    };
                    if same && matches {
                        // Own write or an unrelated event.
                    } else if same && dirty {
                        self.report(format!("{path} changed on disk; unsaved edits kept"), cx);
                        self.refresh_panes(state, cx);
                        cx.notify();
                        return;
                    } else if same {
                        self.editor.update(cx, |e, cx| e.reload(text, cx));
                    } else {
                        self.editor
                            .update(cx, |e, cx| e.set_text(path.to_string(), text.clone(), cx));
                        self.restore_folds(path, &text, state, cx);
                    }
                    // Nothing about a build time reaches Obsidian's status
                    // bar; the number is still there to be read with
                    // `SILICA_DEBUG_STATUS` set.
                    self.status = if debug_status() {
                        format!("{path}  ·  reading view built in {build_ms:.1} ms").into()
                    } else {
                        path.to_string().into()
                    };
                }
                Err(err) => self.unreadable(path, previous, &err, state, cx),
            },
            // A PDF is drawn by its own view, like a canvas and a base. The
            // view never writes, so a change to the file is always somebody
            // else's: `after_changes` forgets the parsed file and reloads
            // every view holding it before this arrives, and what reaches
            // here for the file already on screen is a tab switch back to
            // it, which keeps the page and zoom the view is on. A file the
            // view showed earlier this session comes back to its page: the
            // view remembers, since the leaf records nothing but the file.
            FileKind::Pdf => {
                // Nothing of a note may show through: the reading view is
                // emptied and the editor let go of, as for a canvas.
                self.reading.update(cx, |r, cx| {
                    r.set_note(
                        None,
                        reading::build_with("", "", state.as_ref(), state.build_options()),
                        cx,
                    )
                });
                self.release_buffer(cx);
                if self.pdf.read(cx).path() != Some(path) {
                    let absolute = state.vault().absolute(path);
                    self.pdf.update(cx, |p, cx| p.load(path, absolute, cx));
                }
                self.status = path.to_string().into();
            }
            FileKind::Image => {
                let model = reading::build_with(
                    path,
                    &format!("![[{path}]]"),
                    state.as_ref(),
                    state.build_options(),
                );
                self.reading
                    .update(cx, |r, cx| r.set_note(Some(path.to_string()), model, cx));
                self.release_buffer(cx);
                self.status = path.to_string().into();
            }
            other => {
                // Nothing of the note that was open may survive into a file
                // no view can draw: its text is not this file's and its links
                // would still be clickable underneath the placeholder. The
                // file's own bytes must not reach the editor either — that
                // buffer saves through the markdown writer.
                self.reading.update(cx, |r, cx| {
                    r.set_note(
                        None,
                        reading::build_with("", "", state.as_ref(), state.build_options()),
                        cx,
                    )
                });
                self.release_buffer(cx);
                self.status = format!("{path}: {other:?} files are not supported yet").into();
            }
        }
        self.refresh_panes(state, cx);
        cx.notify();
    }

    /// A file `show_file` could not read. `current` may not name a file
    /// the app could not read: the panes, the explorer row and the autosave
    /// all key off it while the editor still holds another note, and that
    /// mismatch is what silently drops the edits typed into it. The view
    /// goes back to the note it is really showing, or lets go of the buffer
    /// when there is none.
    fn unreadable(
        &mut self,
        path: &str,
        previous: Option<String>,
        err: &std::io::Error,
        state: &Arc<VaultState>,
        cx: &mut Context<Self>,
    ) {
        self.report(format!("Could not read {path}: {err}"), cx);
        self.current = previous.filter(|p| p.as_str() != path);
        match self.current.clone() {
            Some(back) => {
                self.explorer.update(cx, |e, cx| e.follow(&back, cx));
                self.graph.update(cx, |g, cx| g.set_current(Some(back), cx));
            }
            None => self.detach_current(true, state, cx),
        }
    }

    /// Lets go of the canvas on screen, if any. Its edits were flushed by
    /// whoever is changing the file (every route in calls `flush_save`
    /// first); what is left is data describing a file that is no longer
    /// the one shown, and `main_view` would keep painting it.
    fn release_canvas(&mut self, cx: &mut Context<Self>) {
        if self.canvas.read(cx).path().is_some() {
            self.canvas.update(cx, |c, cx| c.release(cx));
        }
    }

    /// Lets go of the base on screen, if any, the way `release_canvas` lets
    /// go of a canvas. Nothing is flushed first: the view never writes.
    fn release_bases(&mut self, cx: &mut Context<Self>) {
        if self.bases.read(cx).path().is_some() {
            self.bases.update(cx, |b, cx| b.release(cx));
        }
    }

    /// Lets go of the PDF on screen, if any, the same way. Nothing is
    /// flushed first: the view never writes.
    fn release_pdf(&mut self, cx: &mut Context<Self>) {
        if self.pdf.read(cx).path().is_some() {
            self.pdf.update(cx, |p, cx| p.release(cx));
        }
    }

    /// Plain text of a cached heading (markup stripped).
    fn heading_text(raw: &str) -> String {
        let doc = silica_markdown::parse(raw);
        match doc.blocks.first().map(|b| &b.kind) {
            Some(silica_markdown::BlockKind::Paragraph { content }) => {
                silica_markdown::plain_text(content)
            }
            _ => raw.to_string(),
        }
        .trim()
        .to_string()
    }

    /// Lets go of the buffer on the way into a file the editor cannot save:
    /// a canvas, an image, a PDF. Unsaved edits are not thrown away — they
    /// would exist nowhere else — so a dirty buffer stays on screen, marked
    /// as belonging to no file; a clean one goes.
    fn release_buffer(&mut self, cx: &mut Context<Self>) {
        let (dirty, name) = {
            let editor = self.editor.read(cx);
            (editor.is_dirty(), editor.name.clone())
        };
        if dirty && !name.is_empty() {
            self.save_task = None;
            self.detached = Some(name);
            return;
        }
        self.clear_buffer(cx);
    }

    /// Empties the editor and forgets what it was showing. Keeping the bytes
    /// of a canvas or a PDF in the editor would let the autosave write them
    /// back through the markdown writer.
    fn clear_buffer(&mut self, cx: &mut Context<Self>) {
        self.save_task = None;
        self.detached = None;
        self.editor
            .update(cx, |e, cx| e.set_text("", String::new(), cx));
        self.words = 0;
        self.chars = 0;
    }

    /// Lets go of the open note: nothing may write the buffer back, and no
    /// pane, graph or history entry may keep describing a file that is not
    /// there. `keep_buffer` is for a file that went away underneath the app,
    /// where the text on screen may hold edits that exist nowhere else; it
    /// stays, marked as saved nowhere, rather than being thrown away.
    fn detach_current(
        &mut self,
        keep_buffer: bool,
        state: &Arc<VaultState>,
        cx: &mut Context<Self>,
    ) {
        self.save_task = None;
        if keep_buffer {
            let name = self.editor.read(cx).name.clone();
            self.detached = (!name.is_empty()).then_some(name);
        } else {
            self.clear_buffer(cx);
        }
        self.release_canvas(cx);
        self.release_bases(cx);
        self.release_pdf(cx);
        self.reading.update(cx, |r, cx| {
            r.set_note(
                None,
                reading::build_with("", "", state.as_ref(), state.build_options()),
                cx,
            )
        });
        self.current = None;
        // Nothing is open, so no row is the open file's: a highlight left
        // behind outlives the note it marked, and the next `set_tree` would
        // keep painting a path the vault no longer has.
        self.explorer.update(cx, |e, cx| e.clear_active(cx));
        self.graph.update(cx, |g, cx| g.set_current(None, cx));
        self.clear_panes(cx);
    }

    /// Drops every row that describes the open note. The panes are not
    /// refreshed on the way out of a file, so without this the outline,
    /// backlinks and outgoing links of a note that is gone stay on screen
    /// and stay clickable.
    fn clear_panes(&mut self, cx: &mut Context<Self>) {
        self.outline.clear();
        self.outline_state.collapsed.clear();
        self.outline_state.file = None;
        self.outline_state.active = None;
        self.backlink_sections.clear();
        self.outgoing_sections.clear();
        let current = self.current.clone();
        self.bookmarks.update(cx, |b, cx| b.set_active(current, cx));
        cx.notify();
    }

    /// Forgets `path`, and anything under it when it is a folder, so back
    /// and forward can never navigate to a file the vault no longer has.
    fn forget_history(&mut self, path: &str) {
        self.history_ix = prune_history(&mut self.history, self.history_ix, path);
        for tab in self.stashed_tabs_mut() {
            tab.history_ix = prune_history(&mut tab.history, tab.history_ix, path);
        }
    }

    fn refresh_panes(&mut self, state: &Arc<VaultState>, cx: &mut Context<Self>) {
        let Some(path) = self.current.clone() else {
            self.clear_panes(cx);
            return;
        };
        self.bookmarks
            .update(cx, |b, cx| b.set_active(Some(path.clone()), cx));
        let cache = state.cache();
        let texts = state.texts();
        self.outline = cache.file(&path).map(outline_headings).unwrap_or_default();
        // Folds belong to the note: another note's outline opens unfolded.
        if self.outline_state.file.as_deref() != Some(path.as_str()) {
            self.outline_state.collapsed.clear();
            self.outline_state.file = Some(path.clone());
            self.outline_state.scroll_pending = true;
        }

        // Linked mentions: every link into this file, grouped by the note
        // it was written in; Links: the ones this file writes, properties
        // included. Unlinked mentions take a search of every note and come
        // in from the background executor.
        let extra = self.backlink_state.extra_context;
        let linked = backlinks::linked_mentions(&cache, &texts, &path, extra);
        let links = backlinks::outgoing_links(&cache, &path);
        drop(cache);
        drop(texts);
        self.backlink_sections = vec![linked, Section::empty(backlinks::UNLINKED)];
        self.outgoing_sections = vec![links, Section::empty(backlinks::UNLINKED)];
        self.sort_backlink_sections(state);
        self.compute_mentions(&path, state, cx);
        cx.notify();
    }

    /// Puts the Backlinks pane's groups in its sort order. The time orders
    /// need every file's stat, so those are only gathered for them.
    fn sort_backlink_sections(&mut self, state: &VaultState) {
        let order = self.backlink_state.sort;
        let times: HashMap<String, (u64, u64)> = match order {
            SortOrder::Alphabetical | SortOrder::AlphabeticalReverse => HashMap::new(),
            _ => state
                .vault()
                .files()
                .iter()
                .map(|f| (f.path.clone(), (f.mtime, f.ctime)))
                .collect(),
        };
        let stat = |p: &str| times.get(p).copied().unwrap_or((0, 0));
        for section in &mut self.backlink_sections {
            backlinks::sort_groups(section, order, &stat);
        }
    }

    /// Toggles "Show more context", which changes every match's text, so
    /// the panes are rebuilt.
    fn toggle_backlink_context(&mut self, cx: &mut Context<Self>) {
        self.backlink_state.extra_context = !self.backlink_state.extra_context;
        if let Some(state) = self.state.clone() {
            self.refresh_panes(&state, cx);
        }
        cx.notify();
    }

    /// Toggles "Collapse results": every group follows the flag again.
    fn toggle_backlink_collapse_all(&mut self, cx: &mut Context<Self>) {
        self.backlink_state.collapse_all = !self.backlink_state.collapse_all;
        self.backlink_state.toggled.clear();
        cx.notify();
    }

    /// Toggles the search filter; showing it puts the keyboard in it.
    fn toggle_backlink_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.backlink_state.show_search = !self.backlink_state.show_search;
        if self.backlink_state.show_search {
            let handle = self.backlink_filter.focus_handle(cx);
            window.focus(&handle, cx);
        }
        cx.notify();
    }

    fn set_backlink_sort(&mut self, order: SortOrder, cx: &mut Context<Self>) {
        self.backlink_state.sort = order;
        self.backlink_state.sort_menu_open = false;
        if let Some(state) = self.state.clone() {
            self.sort_backlink_sections(&state);
        }
        cx.notify();
    }

    /// A pane's rows for the state dump, folds included.
    fn pane_rows_json(&self, which: RightTab) -> Vec<Value> {
        let sections = match which {
            RightTab::Outgoing => &self.outgoing_sections,
            _ => &self.backlink_sections,
        };
        section_rows_json(sections, &|ix| self.section_collapsed(which, ix), &|key| {
            self.group_collapsed(which, key)
        })
    }

    /// Whether a pane's section is folded.
    fn section_collapsed(&self, which: RightTab, ix: usize) -> bool {
        match (which, ix) {
            (RightTab::Outgoing, 0) => self.outgoing_state.links_collapsed,
            (RightTab::Outgoing, _) => self.outgoing_state.unlinked_collapsed,
            (_, 0) => self.backlink_state.linked_collapsed,
            (_, _) => self.backlink_state.unlinked_collapsed,
        }
    }

    fn toggle_section(&mut self, which: RightTab, ix: usize, cx: &mut Context<Self>) {
        let flag = match (which, ix) {
            (RightTab::Outgoing, 0) => &mut self.outgoing_state.links_collapsed,
            (RightTab::Outgoing, _) => &mut self.outgoing_state.unlinked_collapsed,
            (_, 0) => &mut self.backlink_state.linked_collapsed,
            (_, _) => &mut self.backlink_state.unlinked_collapsed,
        };
        *flag = !*flag;
        cx.notify();
    }

    /// Whether a group is folded: the Backlinks pane's "Collapse results"
    /// sets the default and a click on the group inverts it.
    fn group_collapsed(&self, which: RightTab, key: &str) -> bool {
        match which {
            RightTab::Outgoing => self.outgoing_state.toggled.contains(key),
            _ => self.backlink_state.collapse_all ^ self.backlink_state.toggled.contains(key),
        }
    }

    fn toggle_group(&mut self, which: RightTab, key: String, cx: &mut Context<Self>) {
        let toggled = match which {
            RightTab::Outgoing => &mut self.outgoing_state.toggled,
            _ => &mut self.backlink_state.toggled,
        };
        if !toggled.remove(&key) {
            toggled.insert(key);
        }
        cx.notify();
    }

    /// The Link button: rewrites the unlinked mention of `target` at
    /// `mention` in `source` as a link.
    ///
    /// The open note is edited through its buffer, so the change is
    /// undoable and saved like any other edit. Any other note is written
    /// straight to disk, provided the file is still the text the mention
    /// was found in: the range is into those bytes, and another app may
    /// have saved since.
    fn link_mention(
        &mut self,
        source: String,
        target: String,
        mention: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let Some(text) = state.texts().get(&source).cloned() else {
            return;
        };
        let Some(mentioned) = text.get(mention.clone()) else {
            return;
        };
        let markdown = state.app().use_markdown_links.unwrap_or(false);
        let link = backlinks::link_for_mention(
            state.cache().resolver(),
            &target,
            &source,
            mentioned,
            state.link_format(),
            markdown,
        );
        let in_buffer = self.current.as_deref() == Some(source.as_str())
            && self.editor.read(cx).name.as_ref() == source.as_str();
        if in_buffer {
            if self.editor.read(cx).text() != text.as_str() {
                self.report(format!("{source} has unsaved edits; not linked"), cx);
                return;
            }
            self.editor.update(cx, |e, cx| {
                e.set_selection_range(mention.start, mention.end, window, cx);
                e.insert_at_cursors(&link, window, cx);
            });
            return;
        }
        let absolute = match state.vault().write_path(&source) {
            Ok(path) => path,
            Err(err) => {
                self.report(format!("{source} not linked: {err}"), cx);
                return;
            }
        };
        match std::fs::read_to_string(&absolute) {
            Ok(on_disk) if on_disk == *text => {}
            Ok(_) => {
                self.report(format!("{source} changed on disk; not linked"), cx);
                return;
            }
            Err(err) => {
                self.report(format!("Could not read {source}: {err}"), cx);
                return;
            }
        }
        let mut written = text.to_string();
        written.replace_range(mention, &link);
        if let Err(err) = silica_vault::write_no_follow(&absolute, written.as_bytes()) {
            self.report(format!("Could not write {source}: {err}"), cx);
            return;
        }
        self.files_changed(vec![absolute], cx);
    }

    /// Rebuilds the Outline from the editor's buffer instead of from the
    /// index.
    ///
    /// The index is a save behind while a note is being typed, so the rows
    /// name headings the file no longer has — and, worse, carry byte offsets
    /// into a version of the note that is not on screen, which is what makes
    /// clicking one land the caret in the wrong place. Only the Outline is
    /// rebuilt: backlinks and tags are about other files and cannot go stale
    /// from typing in this one. It costs a parse of the buffer, so it is
    /// paid only while the pane showing those offsets is up.
    fn refresh_outline_from_buffer(&mut self, cx: &mut Context<Self>) {
        if !self.right_visible || self.right_tab != RightTab::Outline {
            return;
        }
        let Some(path) = self.current.clone() else {
            return;
        };
        let rows = {
            let editor = self.editor.read(cx);
            // A clean buffer is the file the index read, and a buffer
            // holding another note says nothing about this one.
            if editor.name.as_ref() != path.as_str() || !editor.is_dirty() {
                return;
            }
            outline_headings(&index_markdown(editor.text()))
        };
        self.outline = rows;
        cx.notify();
    }

    /// Finds unlinked mentions in both directions on the background executor.
    fn compute_mentions(&mut self, path: &str, state: &Arc<VaultState>, cx: &mut Context<Self>) {
        self.mention_generation += 1;
        let generation = self.mention_generation;
        let state = state.clone();
        let on_ui = state.clone();
        let path = path.to_string();
        let finder = self.name_finder.clone();
        let extra = self.backlink_state.extra_context;
        cx.spawn(async move |this, cx| {
            let (rows, finder) = cx
                .background_executor()
                .spawn(async move {
                    let finder = finder.or_else(|| {
                        let names = state.names();
                        let refs: Vec<Name<'_>> = names
                            .iter()
                            .map(|(text, path)| Name { text, path })
                            .collect();
                        Finder::new(&refs).map(Arc::new)
                    });
                    (
                        mention_sections(&state, &path, finder.as_deref(), extra),
                        finder,
                    )
                })
                .await;
            this.update(cx, |ws, cx| {
                ws.name_finder = finder;
                if ws.mention_generation != generation || ws.current.as_deref() != Some(&rows.path)
                {
                    return;
                }
                if let Some(slot) = ws.backlink_sections.get_mut(1) {
                    *slot = rows.incoming;
                }
                if let Some(slot) = ws.outgoing_sections.get_mut(1) {
                    *slot = rows.outgoing;
                }
                ws.sort_backlink_sections(&on_ui);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Scrolls whichever view is showing the open note to a byte offset.
    ///
    /// Both views move. The reading view is the one a reading-mode jump
    /// scrolls, but Live Preview is the default mode, and until this moved the
    /// editor too every outline row, backlink and search hit scrolled a view
    /// nobody was looking at.
    fn reveal_offset(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.reading
            .update(cx, |r, cx| r.scroll_to_offset(offset, cx));
        self.editor.update(cx, |e, cx| e.reveal_offset(offset, cx));
        // An outline row or a backlink moved the caret; the keyboard goes
        // where the caret went, not to the dock row that was clicked.
        self.request_focus_main(cx);
    }

    fn scroll_to_subpath(
        &mut self,
        path: &str,
        subpath: &Subpath<'_>,
        state: &Arc<VaultState>,
        cx: &mut Context<Self>,
    ) {
        // `[[doc.pdf#page=3]]`: the link parser reads `page=3` as a heading,
        // and the PDF view reads it as a page. Every `#` part is put back
        // and parsed as pdf.js's hash.
        if FileKind::from_path(path) == FileKind::Pdf {
            if let Subpath::Headings(names) = subpath {
                let raw = names.join("#");
                // Observed: `#page=99` on a five-page file, `#page=0` and
                // `#page=abc` leave the view where it is.
                if let Some(page) = silica_pdf::parse_subpath(&raw).page {
                    if self.pdf.read(cx).path() == Some(path) {
                        self.pdf.update(cx, |p, cx| p.go_to_page(page, false, cx));
                    }
                }
            }
            return;
        }
        let cache = state.cache();
        let Some(meta) = cache.file(path) else {
            return;
        };
        let offset = match subpath {
            Subpath::Headings(names) => {
                let Some(last) = names.last() else { return };
                meta.headings
                    .iter()
                    .find(|h| Self::heading_text(&h.heading).eq_ignore_ascii_case(last.trim()))
                    .map(|h| h.position.start.offset)
            }
            Subpath::Block(id) => meta
                .blocks
                .get(&id.to_lowercase())
                .or_else(|| meta.blocks.get(*id))
                .map(|b| b.position.start.offset),
        };
        drop(cache);
        if let Some(offset) = offset {
            self.reveal_offset(offset, cx);
        }
    }

    fn follow_link(&mut self, target: LinkTarget, cx: &mut Context<Self>) {
        let new_tab = std::mem::take(&mut self.link_in_new_tab);
        match target {
            LinkTarget::Internal(body) => {
                let Some(state) = self.state.clone() else {
                    return;
                };
                let source = self.current.clone().unwrap_or_default();
                let parsed = silica_markdown::parse_linktext(&body);
                let resolved = state.cache().resolve(&body, &source).map(str::to_string);
                match resolved {
                    Some(path) if new_tab => self.open_in_new_tab(path, parsed.subpath, cx),
                    Some(path) => {
                        if path == source {
                            if let Some(sub) = parsed.subpath {
                                self.scroll_to_subpath(&path, &sub, &state, cx);
                            }
                        } else {
                            self.open_file(path, parsed.subpath, cx);
                        }
                    }
                    None => {
                        // Obsidian creates the note a dangling link points at.
                        if new_tab {
                            self.new_tab(cx);
                        }
                        let name = parsed.path.to_string();
                        self.create_note_for(&name, None, cx);
                    }
                }
            }
            LinkTarget::External(url) => cx.open_url(&url),
            LinkTarget::Tag(tag) => self.show_search(&format!("tag:#{tag}"), None, cx),
            LinkTarget::Footnote(n) => {
                // Obsidian's reference is an anchor to that one definition,
                // not to the end of the note. The model knows where each
                // definition is written, so the same reveal every heading
                // jump uses takes the reader there: the Footnotes block in
                // reading, the definition's own line while editing.
                let at = self
                    .reading
                    .read(cx)
                    .model()
                    .footnotes
                    .get(n.saturating_sub(1))
                    .copied();
                match at {
                    Some(offset) => self.reveal_offset(offset, cx),
                    None => self.reading.update(cx, |r, cx| r.scroll_to_end(cx)),
                }
            }
        }
    }

    fn toggle_mode(&mut self, _: &ToggleMode, window: &mut Window, cx: &mut Context<Self>) {
        let mode = match self.mode {
            ViewMode::Reading => self.editing_mode(),
            ViewMode::LivePreview | ViewMode::Source | ViewMode::Graph => ViewMode::Reading,
        };
        self.set_mode(mode, window, cx);
    }

    fn go_back(&mut self, _: &GoBack, _: &mut Window, cx: &mut Context<Self>) {
        self.record_history_state(cx);
        if self.history_ix > 1 {
            self.history_ix -= 1;
            self.navigate_history(cx);
        }
    }

    fn go_forward(&mut self, _: &GoForward, _: &mut Window, cx: &mut Context<Self>) {
        self.record_history_state(cx);
        if self.history_ix < self.history.len() {
            self.history_ix += 1;
            self.navigate_history(cx);
        }
    }

    fn navigate_history(&mut self, cx: &mut Context<Self>) {
        let (Some(state), Some(entry)) = (
            self.state.clone(),
            self.history.get(self.history_ix.wrapping_sub(1)).cloned(),
        ) else {
            return;
        };
        if self.mode == ViewMode::Graph {
            self.mode = self.mode_before_graph;
        }
        self.flush_save(cx);
        // Obsidian's history carries the view state, so back and forward
        // restore the mode the note was left in as well as the note.
        self.mode = entry.mode;
        self.show_file(&entry.path, &state, cx);
        if entry.offset > 0 {
            self.reveal_offset(entry.offset, cx);
        }
        self.request_focus_main(cx);
    }

    /// Writes the view state into the entry the history is standing on,
    /// which is the note about to be left.
    ///
    /// The offset is what brings the reader back to the same place: the
    /// caret while editing, and the block at the top of the page in
    /// Reading, where there is no caret to speak of.
    fn record_history_state(&mut self, cx: &mut Context<Self>) {
        let Some(current) = self.current.clone() else {
            return;
        };
        let mode = if self.mode == ViewMode::Graph {
            self.mode_before_graph
        } else {
            self.mode
        };
        let offset = if mode == ViewMode::Reading {
            self.reading.read(cx).top_offset().unwrap_or(0)
        } else {
            self.editor.read(cx).selection().start
        };
        let Some(entry) = self.history.get_mut(self.history_ix.wrapping_sub(1)) else {
            return;
        };
        if entry.path == current {
            entry.mode = mode;
            entry.offset = offset;
        }
    }

    /// A task checkbox a reader clicked in the reading view. The editor
    /// still holds the open note's buffer while the reader is the view on
    /// screen, so the edit takes exactly the path a Live Preview click
    /// takes: one undo step, and the autosave writes it back. The reader
    /// is then rebuilt from that buffer rather than from the file, which
    /// the save has not reached yet.
    fn toggle_task(&mut self, path: &str, marker: TaskMarker, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        // A click that outlived the note it was made in must write nothing:
        // the marker's line and column belong to that note's bytes.
        if self.current.as_deref() != Some(path) || self.editor.read(cx).name.as_ref() != path {
            return;
        }
        if !self.editor.update(cx, |e, cx| {
            e.toggle_task_at(marker.line, marker.col, None, cx)
        }) {
            return;
        }
        let text = self.editor.read(cx).text().to_string();
        self.words = text.split_whitespace().count();
        self.chars = text.chars().count();
        let model = reading::build_with(path, &text, state.as_ref(), state.build_options());
        self.reading
            .update(cx, |r, cx| r.set_note(Some(path.to_string()), model, cx));
        cx.notify();
    }

    /// The reading view edited a property in the reading view's properties
    /// block. Read the vault file, plan the change against its frontmatter,
    /// write it back, and rebuild the reading model so the change is visible.
    fn change_property(
        &mut self,
        path: &str,
        changes: &[Change],
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state.clone() else {
            return;
        };
        // Only the currently open note can be edited this way.
        if self.current.as_deref() != Some(path) {
            return;
        }
        let absolute = match self.vault_path(path) {
            Ok(p) => p,
            Err(_) => return,
        };
        let Ok(text) = std::fs::read_to_string(&absolute) else {
            return;
        };
        let Some(props) = silica_cache::note_properties(&text) else {
            return;
        };
        let Some(edits) = silica_formats::properties::plan(&text, &props, changes) else {
            return;
        };
        let mut new_text = text;
        // Apply edits in reverse order so earlier byte ranges stay valid.
        for edit in edits.iter().rev() {
            new_text.replace_range(edit.start..edit.end, &edit.text);
        }
        if let Err(e) = std::fs::write(&absolute, &new_text) {
            self.report(format!("could not write properties: {e}"), cx);
            return;
        }
        let model = reading::build_with(path, &new_text, state.as_ref(), state.build_options());
        self.reading
            .update(cx, |r, cx| r.set_note(Some(path.to_string()), model, cx));
        cx.notify();
    }

    /// Writes the editor's buffer back after an edit settles (two seconds,
    /// restarted by every edit, the way Obsidian's autosave works).
    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        self.save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            this.update(cx, |ws, cx| ws.flush_save(cx)).ok();
        }));
    }

    /// Writes the editor's buffer to its file now, if it has unsaved edits.
    /// The bytes are the file's own line endings and BOM around the edited
    /// text, so nothing but the edit changes on disk.
    pub fn flush_save(&mut self, cx: &mut Context<Self>) {
        self.save_task = None;
        let Some(state) = self.state.clone() else {
            return;
        };
        self.flush_canvas_save(&state, cx);
        self.flush_bases_save(&state, cx);
        let (dirty, name) = {
            let editor = self.editor.read(cx);
            (editor.is_dirty(), editor.name.to_string())
        };
        // Only the open note is written back; a buffer that is not a vault
        // file (the synthetic stress note) never touches the disk.
        if !dirty || self.current.as_deref() != Some(name.as_str()) {
            return;
        }
        let contents = self.editor.read(cx).file_contents();
        // The one place a note's bytes are written: the path is checked
        // for links and the open does not follow one, so a save can never
        // land outside the vault.
        let absolute = match state.vault().write_path(&name) {
            Ok(path) => path,
            Err(err) => {
                self.report(format!("{name} not saved: {err}"), cx);
                return;
            }
        };
        // The vault is the source of truth. If the file is not the bytes
        // this note was loaded from or last saved as, something else wrote
        // it — Obsidian, in the mixed use the phase 3 gate is about — and
        // overwriting it would throw that away. A file that cannot be read
        // at all is the same answer only more so: a note deleted underneath
        // the app must not be recreated here from the buffer it left behind.
        // The buffer keeps its edits and says so instead.
        match std::fs::read_to_string(&absolute) {
            Ok(on_disk) if self.editor.read(cx).disk_unchanged(&on_disk) => {}
            Ok(_) => {
                self.report(
                    format!("{name} changed on disk; not overwritten, your edits are still here"),
                    cx,
                );
                return;
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                self.report(
                    format!("{name} was deleted; not recreated, your edits are still here"),
                    cx,
                );
                return;
            }
            Err(err) => {
                self.report(
                    format!("Could not read {name}: {err}; not overwritten, your edits are kept"),
                    cx,
                );
                return;
            }
        }
        match silica_vault::write_no_follow(&absolute, contents.as_bytes()) {
            Ok(()) => {
                self.editor.update(cx, |e, _| e.mark_saved(&contents));
                self.report(format!("{name} saved"), cx);
            }
            Err(err) => {
                self.report(format!("Could not save {name}: {err}"), cx);
            }
        }
        cx.notify();
    }

    /// "Copy search results" from the Search pane's count menu.
    fn open_copy_results(
        &mut self,
        entries: Vec<CopyEntry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_editor_suggest(cx);
        self.copy_results.update(cx, |m, cx| m.open(entries, cx));
        self.modal = Modal::CopyResults;
        window.focus(&self.copy_results.focus_handle(cx), cx);
        cx.notify();
    }

    // ----- file operations -------------------------------------------------------

    fn open_prompt(
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
    fn copy_path(&mut self, path: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(path.to_string()));
    }

    /// The explorer menu's "Open in default app": the file handed to the
    /// desktop, one command per platform and no crate for it, so the
    /// dependency tree is unchanged. The child is spawned and let go -- the
    /// handler it starts outlives this process on every platform -- and only a
    /// failure to spawn is reported, since nothing after that is Silica's to
    /// know.
    fn open_in_default_app(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let absolute = state.vault().absolute(path);
        let mut command = if cfg!(target_os = "macos") {
            std::process::Command::new("open")
        } else if cfg!(target_os = "windows") {
            // `start` is a shell builtin rather than a program, and its first
            // quoted argument is the window title, not the file.
            let mut c = std::process::Command::new("cmd");
            c.args(["/C", "start", ""]);
            c
        } else {
            std::process::Command::new("xdg-open")
        };
        if let Err(err) = command
            .arg(&absolute)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            self.report(format!("Could not open {path}: {err}"), cx);
        }
    }

    /// The explorer menu's "Search in folder": the Search pane scoped with the
    /// `path:` operator the query grammar already has. The trailing slash
    /// keeps `Notes/` from also matching `Notes archive/`. What Obsidian puts
    /// in its own box here was not captured, so the query is marked OBSERVE in
    /// [[Specification#The file explorer's context menus]].
    fn search_in_folder(&mut self, folder: &str, window: &mut Window, cx: &mut Context<Self>) {
        let query = match folder.is_empty() {
            true => String::new(),
            false => format!("path:\"{folder}/\""),
        };
        self.show_search(&query, Some(window), cx);
    }

    fn report(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
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
    fn new_note(&mut self, folder: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let path = state.vault().available_path(folder, "Untitled", "md");
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
    fn new_canvas(&mut self, folder: &str, cx: &mut Context<Self>) {
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
    fn new_base(&mut self, folder: &str, cx: &mut Context<Self>) {
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
    fn add_base_view(&mut self, cx: &mut Context<Self>) {
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
    fn change_base_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        // A base that did not parse has no views to switch between, and a
        // chooser of nothing is not what the command does.
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
    fn copy_base_table(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
    fn insert_base(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        // The link is the file's own name with its extension, the way a
        // non-markdown file is always linked; the new base is in the vault
        // root, as `bases:new-file`'s is.
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
    fn add_base_item(&mut self, cx: &mut Context<Self>) {
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
    fn set_note_property(
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
        // Any group's editor may be holding the note, not just the active
        // one, and a buffer that differs from the disk is unsaved work.
        if self.buffer_differs(&path, &on_disk, cx) {
            self.report(format!("{path} has unsaved edits; {key} not set"), cx);
            return;
        }
        let written = {
            let types = state.property_types();
            crate::bases::edit::write_property(&on_disk, &key, &value, &types)
        };
        // An unchanged value writes nothing at all, which is what keeps a
        // committed field from touching a file it did not change.
        let Some(written) = written else {
            return;
        };
        if let Err(err) = silica_vault::write_no_follow(&absolute, written.as_bytes()) {
            self.report(format!("Could not write {path}: {err}"), cx);
            return;
        }
        self.report(format!("{key} set in {path}"), cx);
        // The index is what every base resolves against, so the table comes
        // back through the same path an outside edit would take.
        self.files_changed(vec![absolute], cx);
    }

    /// Whether any group's editor holds `path` with bytes that are not
    /// `on_disk` — that is, with unsaved edits a write would discard.
    fn buffer_differs(&self, path: &str, on_disk: &str, cx: &mut Context<Self>) -> bool {
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
    fn flush_bases_save(&mut self, state: &Arc<VaultState>, cx: &mut Context<Self>) {
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
    fn flush_canvas_save(&mut self, state: &Arc<VaultState>, cx: &mut Context<Self>) {
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
    fn record_canvas_view_state(
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
    fn create_note_for(&mut self, name: &str, window: Option<&mut Window>, cx: &mut Context<Self>) {
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

    fn create_from_link(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.create_note_for(name, Some(window), cx);
    }

    fn start_rename(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
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

    fn start_move(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
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

    fn start_new_folder(&mut self, parent: &str, window: &mut Window, cx: &mut Context<Self>) {
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

    fn start_delete(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
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

    fn delete_now(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let was_current = self.current.as_deref() == Some(path)
            || self
                .current
                .as_deref()
                .is_some_and(|c| c.starts_with(&format!("{path}/")));
        if was_current {
            // Nothing may write the buffer back into a file that is gone,
            // and no pane may go on describing it.
            self.detach_current(false, &state, cx);
            // The main area may stop painting the view that was showing the
            // note, and a focus left on it would sit outside the dispatch
            // tree and swallow every key. The same rule as everywhere else
            // decides where it goes instead: the workspace root when nothing
            // is painted, the detached buffer when one stayed behind.
            self.focus_main(window, cx);
        }
        self.forget_history(path);
        match state.trash(path) {
            // Obsidian shows nothing for a delete that worked.
            Ok(changes) => self.after_changes(changes, cx),
            Err(err) => self.report(err, cx),
        }
    }

    fn duplicate(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        match state.duplicate(path) {
            // Nothing is reported for a copy that worked either; the new
            // file opening is the feedback.
            Ok((copy, changes)) => {
                self.after_changes(changes, cx);
                self.open_file(copy, None, cx);
            }
            Err(err) => self.report(err, cx),
        }
    }

    /// Enter in the prompt: finish the pending operation with `value`.
    fn prompt_submitted(&mut self, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(op) = self.pending.clone() else {
            self.close_modal(window, cx);
            return;
        };
        let value = value.trim();
        match op {
            PendingOp::Rename(from) => {
                // The box holds the stem, which is what `renamed_path`
                // takes -- the same path and the same validator the view
                // header's inline rename field commits through.
                let to = match self.renamed_path(&from, value) {
                    Ok(to) => to,
                    Err(err) => {
                        self.prompt.update(cx, |p, cx| p.set_error(err, cx));
                        return;
                    }
                };
                self.rename_with_links(&from, &to, window, cx);
            }
            PendingOp::Move(from) => {
                let folder = value.trim_matches('/');
                let name = from.rsplit('/').next().unwrap_or(&from);
                let to = if folder.is_empty() {
                    name.to_string()
                } else {
                    format!("{folder}/{name}")
                };
                self.rename_with_links(&from, &to, window, cx);
            }
            PendingOp::Delete(path) => {
                self.pending = None;
                self.close_modal(window, cx);
                self.delete_now(&path, window, cx);
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
            PendingOp::UpdateLinks { from, to } => self.finish_rename(&from, &to, true, window, cx),
            PendingOp::AddBookmark(draft) => {
                self.pending = None;
                self.close_modal(window, cx);
                self.finish_bookmark(draft, value, cx);
            }
            // Save on "Edit bookmark" changes nothing, as Obsidian's did.
            PendingOp::EditBookmark(_) => {
                self.pending = None;
                self.close_modal(window, cx);
            }
        }
    }

    /// The prompt's secondary button: "Don't update" the links.
    fn prompt_secondary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.pending.clone() {
            Some(PendingOp::UpdateLinks { from, to }) => {
                self.finish_rename(&from, &to, false, window, cx);
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

    // ----- bookmarks ---------------------------------------------------------------

    /// Opens "Add bookmark" for a draft: the Title field, empty, with the
    /// name the bookmark gets without one as its placeholder.
    fn start_bookmark(
        &mut self,
        draft: BookmarkDraft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let placeholder = draft.placeholder();
        let (title, detail) = match &draft {
            BookmarkDraft::Tabs(paths) => (
                format!("Add {} bookmarks", paths.len()),
                Some("Title".to_string()),
            ),
            BookmarkDraft::Graph => ("Add bookmark".to_string(), Some("Title".to_string())),
            BookmarkDraft::Search(query) => {
                ("Add bookmark".to_string(), Some(format!("Query: {query}")))
            }
            BookmarkDraft::File { .. } | BookmarkDraft::Folder(_) => (
                "Add bookmark".to_string(),
                Some(format!("Path: {placeholder}")),
            ),
        };
        let initial = match &draft {
            BookmarkDraft::Graph => placeholder.clone(),
            _ => String::new(),
        };
        self.open_prompt(
            PendingOp::AddBookmark(draft),
            PromptSpec {
                title: title.into(),
                detail: detail.map(Into::into),
                input: Some(initial),
                placeholder: placeholder.into(),
                confirm: "Save".into(),
                ..PromptSpec::default()
            },
            window,
            cx,
        );
    }

    /// `bookmarks:bookmark-current-view` and the pane's first button: the
    /// open note, or the graph while it is up. A note already bookmarked
    /// opens "Edit bookmark", where Remove is the only change offered.
    fn start_bookmark_current_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode == ViewMode::Graph {
            self.start_bookmark(BookmarkDraft::Graph, window, cx);
            return;
        }
        let Some(path) = self.current.clone() else {
            self.report("No file is open", cx);
            return;
        };
        self.start_bookmark_path(&path, window, cx);
    }

    /// "Bookmark..." for a file or folder, from the explorer's menu or the
    /// open note.
    fn start_bookmark_path(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        if state.vault().absolute(path).is_dir() {
            self.start_bookmark(BookmarkDraft::Folder(path.to_string()), window, cx);
            return;
        }
        if state.bookmarks().has_file(path) {
            let placeholder = Bookmark::file(path, None, None, 0).display_title();
            self.open_prompt(
                PendingOp::EditBookmark(path.to_string()),
                PromptSpec {
                    title: "Edit bookmark".into(),
                    detail: Some(format!("Path: {placeholder}").into()),
                    input: Some(String::new()),
                    placeholder: placeholder.into(),
                    confirm: "Save".into(),
                    secondary: Some("Remove".into()),
                    ..PromptSpec::default()
                },
                window,
                cx,
            );
            return;
        }
        self.start_bookmark(
            BookmarkDraft::File {
                path: path.to_string(),
                subpath: None,
            },
            window,
            cx,
        );
    }

    /// "New group": an empty group with the name typed.
    fn start_new_bookmark_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.start_bookmark(BookmarkDraft::Tabs(Vec::new()), window, cx);
    }

    /// The prompt answered: the item for the draft, with the typed title if
    /// any, appended and written.
    fn finish_bookmark(&mut self, draft: BookmarkDraft, title: &str, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let title = (!title.trim().is_empty()).then(|| title.trim().to_string());
        let ctime = now_millis();
        let item = match draft {
            BookmarkDraft::File { path, subpath } => {
                Bookmark::file(&path, subpath.as_deref(), title.as_deref(), ctime)
            }
            BookmarkDraft::Folder(path) => Bookmark::folder(&path, title.as_deref(), ctime),
            BookmarkDraft::Search(query) => Bookmark::search(&query, title.as_deref(), ctime),
            BookmarkDraft::Graph => {
                let config = state.vault().graph_config().unwrap_or_default();
                Bookmark::graph(
                    title.as_deref().unwrap_or("Untitled Graph"),
                    crate::graph::full_document(&config),
                    ctime,
                )
            }
            BookmarkDraft::Tabs(paths) => {
                // Every member shares one time, the group's is its own, as
                // "Bookmark all tabs" writes them.
                let members: Vec<Bookmark> = paths
                    .iter()
                    .map(|p| Bookmark::file(p, None, None, ctime))
                    .collect();
                let name = title.unwrap_or_else(|| BookmarkDraft::Tabs(Vec::new()).placeholder());
                Bookmark::group(Some(&name), members, now_millis())
            }
        };
        self.change_bookmarks(cx, move |b| b.push(item));
    }

    /// Applies a change to the bookmarks, writes the file and redraws the
    /// pane; a write that fails leaves both as they were and says so.
    fn change_bookmarks(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Bookmarks)) {
        let Some(state) = self.state.clone() else {
            return;
        };
        if let Err(err) = state.update_bookmarks(change) {
            self.report(format!("bookmarks.json could not be written: {err}"), cx);
            return;
        }
        self.refresh_bookmarks_pane(cx);
    }

    fn refresh_bookmarks_pane(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let items = state.bookmarks().items.clone();
        let current = self.current.clone();
        self.bookmarks.update(cx, |b, cx| {
            b.set_items(items, cx);
            b.set_active(current, cx);
        });
    }

    /// Renames, rewriting links right away or asking first when
    /// `alwaysUpdateLinks` is off and there are links to rewrite.
    fn rename_with_links(
        &mut self,
        from: &str,
        to: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state.clone() else {
            return;
        };
        if from == to {
            self.pending = None;
            self.close_modal(window, cx);
            return;
        }
        if state.update_links_automatically() {
            self.finish_rename(from, to, true, window, cx);
            return;
        }
        let (files, links) = state.rename_preview(from, to);
        if links == 0 {
            self.finish_rename(from, to, false, window, cx);
            return;
        }
        let name = from.rsplit('/').next().unwrap_or(from);
        self.open_prompt(
            PendingOp::UpdateLinks {
                from: from.to_string(),
                to: to.to_string(),
            },
            PromptSpec {
                title: "Update links?".into(),
                detail: Some(
                    format!(
                        "{links} link{} in {files} file{} point to {name}. Update them to the new name?",
                        if links == 1 { "" } else { "s" },
                        if files == 1 { "" } else { "s" }
                    )
                    .into(),
                ),
                input: None,
                confirm: "Update links".into(),
                secondary: Some("Don't update".into()),
                ..PromptSpec::default()
            },
            window,
            cx,
        );
    }

    fn finish_rename(
        &mut self,
        from: &str,
        to: &str,
        update_links: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state.clone() else {
            return;
        };
        // The open note's edits must be on disk before its file moves or
        // its links are rewritten.
        self.flush_save(cx);
        match state.rename(from, to, update_links) {
            Ok(outcome) => {
                self.pending = None;
                self.close_modal(window, cx);
                self.apply_rename(outcome, cx);
            }
            Err(err) => self.prompt.update(cx, |p, cx| p.set_error(err, cx)),
        }
    }

    /// Follows the open note and the history to their new paths, then
    /// refreshes everything the way a filesystem event would.
    fn apply_rename(&mut self, outcome: RenameOutcome, cx: &mut Context<Self>) {
        let renamed: HashMap<&str, &str> = outcome
            .renamed
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        // A note's fold record follows it to its new path.
        if let Some(root) = self.state.as_ref().map(|s| s.root.clone()) {
            if let Some(store) = self.fold_store(&root) {
                for (from, to) in &renamed {
                    store.rename(from, to);
                }
            }
        }
        for entry in &mut self.history {
            if let Some(new) = renamed.get(entry.path.as_str()) {
                entry.path = new.to_string();
            }
        }
        for tab in self.stashed_tabs_mut() {
            for entry in &mut tab.history {
                if let Some(new) = renamed.get(entry.path.as_str()) {
                    entry.path = new.to_string();
                }
            }
            if let Some(path) = tab.path.as_mut() {
                if let Some(new) = renamed.get(path.as_str()) {
                    *path = new.to_string();
                }
            }
        }
        // Every group's views follow the rename, the active one's included.
        for group in &self.groups {
            group.editor.update(cx, |e, _| {
                if let Some(new) = renamed.get(e.name.as_ref()) {
                    e.set_name(new.to_string());
                }
            });
            group.reading.update(cx, |r, _| {
                if let Some(new) = r.path.as_deref().and_then(|p| renamed.get(p)) {
                    r.path = Some(new.to_string());
                }
            });
        }
        if let Some(current) = self.current.clone() {
            if let Some(new) = renamed.get(current.as_str()) {
                self.current = Some(new.to_string());
            }
        }
        // Obsidian says nothing about the rename itself. It shows one
        // Notice, and only when links were rewritten:
        // [[Specification#A move that changes the file's basename]] records
        // it as exactly `Updated 1 link in 1 file.`
        let links = (outcome.rewritten_links > 0).then(|| {
            format!(
                "Updated {} link{} in {} file{}.",
                outcome.rewritten_links,
                plural(outcome.rewritten_links),
                outcome.rewritten_files,
                plural(outcome.rewritten_files),
            )
        });
        // A note that changed underneath keeps its own bytes. Obsidian has
        // nothing to say here because it never skips, and silence would
        // leave stale links behind with no sign of it, so this one stays.
        let skipped = match outcome.stale.len() {
            0 => None,
            n => Some(format!(
                "{n} file{} changed on disk and {} left alone",
                plural(n),
                if n == 1 { "was" } else { "were" }
            )),
        };
        self.after_changes(outcome.changes, cx);
        if let Some(current) = self.current.clone() {
            self.explorer.update(cx, |e, cx| e.follow(&current, cx));
        }
        let notice = match (links, skipped) {
            (None, None) => None,
            (Some(l), None) => Some(l),
            (None, Some(s)) => Some(s),
            (Some(l), Some(s)) => Some(format!("{l} {s}")),
        };
        if let Some(notice) = notice {
            self.report(notice, cx);
        }
    }

    /// Renames the open note to `new_name` (same folder, same extension)
    /// right away; `--rename` on the command line, for smoke tests.
    pub fn rename_current(&mut self, new_name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(from) = self.current.clone() else {
            return;
        };
        let folder = from.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        let name = from.rsplit('/').next().unwrap_or(&from);
        let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("md");
        let to = if folder.is_empty() {
            format!("{new_name}.{ext}")
        } else {
            format!("{folder}/{new_name}.{ext}")
        };
        self.finish_rename(&from, &to, true, window, cx);
    }

    /// Opens one of the file prompts for the open note (`--prompt` on the
    /// command line, for screenshots): `rename`, `move`, `delete`, `folder`.
    pub fn open_file_prompt(&mut self, kind: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(current) = self.current.clone() else {
            return;
        };
        match kind {
            "move" => self.start_move(&current, window, cx),
            "delete" => self.start_delete(&current, window, cx),
            "folder" => self.start_new_folder("", window, cx),
            _ => self.start_rename(&current, window, cx),
        }
    }

    /// Which view the main area paints for the mode and the open file. A
    /// view only gets the main area when it has this file to show: an
    /// editor with no file behind it — one left over from a note that is
    /// gone, or one never given a note, as on launch over a vault with
    /// nothing to open — takes edits nobody saves, and a reading view left
    /// behind keeps a deleted note's links on screen and clickable.
    fn main_view(&self, cx: &App) -> MainView {
        let has_note = self.reading.read(cx).path.is_some();
        let image = self
            .current
            .as_deref()
            .is_some_and(|p| FileKind::from_path(p) == FileKind::Image);
        let has_buffer = !self.editor.read(cx).name.is_empty();
        let canvas =
            self.current.is_some() && self.canvas.read(cx).path() == self.current.as_deref();
        let bases = self.current.is_some() && self.bases.read(cx).path() == self.current.as_deref();
        let pdf = self.current.is_some() && self.pdf.read(cx).path() == self.current.as_deref();
        Self::main_view_for(self.mode, has_note, image, has_buffer, canvas, bases, pdf)
    }

    /// The rule itself, over what the views have to show: `has_note` is the
    /// reading view's built note, `image` whether the open file is one,
    /// `has_buffer` whether the editor's buffer names a file, `canvas`
    /// whether the canvas view holds the open file, `bases` whether the
    /// Bases view does and `pdf` whether the PDF view does. None of a
    /// canvas, a base or a PDF has a reading or a source mode, so any of
    /// them is painted in every mode but the graph's.
    fn main_view_for(
        mode: ViewMode,
        has_note: bool,
        image: bool,
        has_buffer: bool,
        canvas: bool,
        bases: bool,
        pdf: bool,
    ) -> MainView {
        match mode {
            ViewMode::Graph => MainView::Graph,
            _ if canvas => MainView::Canvas,
            _ if bases => MainView::Bases,
            _ if pdf => MainView::Pdf,
            ViewMode::Reading if has_note => MainView::Reading,
            // An image has no source to edit, so it reads the same in the
            // editing modes as it does in the reading view.
            ViewMode::LivePreview | ViewMode::Source if image && has_note => MainView::Reading,
            ViewMode::LivePreview | ViewMode::Source if has_buffer => MainView::Editor,
            _ => MainView::Empty,
        }
    }

    /// What one group's rendering reads: the live fields for the active
    /// group, the stashed ones for the others.
    fn group_view(&self, ix: usize) -> GroupView {
        let g = &self.groups[ix];
        if ix == self.active_group {
            return GroupView {
                id: g.id,
                active: true,
                tabs: self.tab_snapshots(),
                active_tab: self.active_tab,
                current: self.current.clone(),
                mode: self.mode,
                history_len: self.history.len(),
                history_ix: self.history_ix,
                detached: self.detached.clone(),
                editor: self.editor.clone(),
                reading: self.reading.clone(),
                canvas: self.canvas.clone(),
                bases: self.bases.clone(),
                pdf: self.pdf.clone(),
            };
        }
        let tab = g.tabs.get(g.active_tab);
        GroupView {
            id: g.id,
            active: false,
            tabs: g.tabs.clone(),
            active_tab: g.active_tab,
            current: tab.and_then(|t| t.path.clone()),
            mode: tab.map(|t| t.mode).unwrap_or(ViewMode::Reading),
            history_len: tab.map(|t| t.history.len()).unwrap_or(0),
            history_ix: tab.map(|t| t.history_ix).unwrap_or(0),
            detached: g.detached.clone(),
            editor: g.editor.clone(),
            reading: g.reading.clone(),
            canvas: g.canvas.clone(),
            bases: g.bases.clone(),
            pdf: g.pdf.clone(),
        }
    }

    /// The main area: the layout tree drawn as nested rows and columns of
    /// groups, a draggable divider between each pair of children.
    fn render_layout(
        &self,
        node: &Layout,
        path: &mut Vec<usize>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        match node {
            Layout::Group(id) => match self.group_ix(*id) {
                Some(ix) => self.render_group(ix, theme, cx),
                None => div().into_any_element(),
            },
            Layout::Split {
                direction,
                children,
                ..
            } => {
                let row = *direction == SplitDirection::Vertical;
                // A split whose children do not all carry a `dimension`
                // shares its room equally, as Obsidian draws one that was
                // never resized.
                let dims: Option<Vec<f64>> =
                    children.iter().map(|c| self.dimension_of(c)).collect();
                let split_path = path.clone();
                let drag_path = split_path.clone();
                let mut container = div()
                    .id(SharedString::from(format!("split-{split_path:?}")))
                    .flex()
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .overflow_hidden()
                    .when(row, |d| d.flex_row())
                    .when(!row, |d| d.flex_col())
                    .on_drag_move(cx.listener(
                        move |this, e: &DragMoveEvent<DividerDrag>, _, cx| {
                            let drag = e.drag(cx).clone();
                            if drag.path == drag_path {
                                this.drag_divider(&drag, e.bounds, e.event.position, cx);
                            }
                        },
                    ));
                for (ix, child) in children.iter().enumerate() {
                    path.push(ix);
                    let body = self.render_layout(child, path, theme, cx);
                    path.pop();
                    let grow = dims.as_ref().map(|d| d[ix] as f32).unwrap_or(1.0);
                    let mut cell = div()
                        .relative()
                        .min_w_0()
                        .min_h_0()
                        .overflow_hidden()
                        .flex_basis(px(0.))
                        .child(body);
                    cell.style().flex_grow = Some(grow.max(0.01));
                    if ix > 0 {
                        let drag = DividerDrag {
                            path: split_path.clone(),
                            before: ix - 1,
                        };
                        let handle = div()
                            .id(("divider", ix as u64))
                            .absolute()
                            .when(row, |d| {
                                d.left(px(0.))
                                    .top_0()
                                    .h_full()
                                    .w(px(6.))
                                    .cursor_col_resize()
                            })
                            .when(!row, |d| {
                                d.top(px(0.))
                                    .left_0()
                                    .w_full()
                                    .h(px(6.))
                                    .cursor_row_resize()
                            })
                            .occlude()
                            .on_drag(drag, |_, _, _, cx| cx.new(|_| NoDragPreview));
                        cell = cell
                            .when(row, |d| d.border_l_1())
                            .when(!row, |d| d.border_t_1())
                            .border_color(theme.border)
                            .child(handle);
                    }
                    container = container.child(cell);
                }
                container.into_any_element()
            }
        }
    }

    /// One group: its tab strip, view header and the view itself. A mouse
    /// down anywhere in a group that is not the active one makes it so
    /// before the view underneath handles the click.
    fn render_group(&self, ix: usize, theme: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let view = self.group_view(ix);
        let id = view.id;
        let mut order = Vec::new();
        self.layout_tree.groups(&mut order);
        // The corner groups: top left is the first group however the
        // splits run, top right needs the tree's shape.
        let first = order.first() == Some(&id);
        let last = self.layout_tree.top_right() == Some(id);
        div()
            .id(("group", id))
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .when(!view.active, |d| {
                d.capture_any_mouse_down(
                    cx.listener(move |this, _, _, cx| this.activate_group(id, cx)),
                )
            })
            .child(self.render_tab_strip(&view, first, last, theme, cx))
            .child(self.render_header(&view, theme, cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(self.render_group_main(&view, theme, cx))
                    .when(self.dragging_tab, |d| {
                        d.child(self.render_drop_zones(id, theme, cx))
                    }),
            )
            .into_any_element()
    }

    /// The drop target over a group's content while a tab is dragged: it
    /// follows the pointer to light the zone the tab would land in — the
    /// whole area for the strip, the near half for a split — and takes the
    /// drop. It covers the view, so nothing under it reacts to the drag.
    fn render_drop_zones(
        &self,
        id: u64,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let zone = self.tab_drop.filter(|(g, _)| *g == id).map(|(_, z)| z);
        let wash = theme.accent.opacity(0.15);
        let edge = theme.accent.opacity(0.6);
        div()
            .id(("drop-zones", id))
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .occlude()
            .on_drag_move(cx.listener(move |this, e: &DragMoveEvent<TabDrag>, _, cx| {
                this.tab_drag_moved(id, e.bounds, e.event.position, cx)
            }))
            .on_drop(cx.listener(move |this, drag: &TabDrag, window, cx| {
                this.drop_tab_on_group(drag, id, window, cx)
            }))
            .when_some(zone, |d, zone| {
                let half = relative(0.5);
                d.child(
                    div()
                        .absolute()
                        .bg(wash)
                        .border_2()
                        .border_color(edge)
                        .rounded(px(6.))
                        .map(|d| match zone {
                            DropZone::Center => d.top_0().left_0().right_0().bottom_0(),
                            DropZone::Left => d.top_0().left_0().bottom_0().w(half),
                            DropZone::Right => d.top_0().right_0().bottom_0().w(half),
                            DropZone::Top => d.top_0().left_0().right_0().h(half),
                            DropZone::Bottom => d.bottom_0().left_0().right_0().h(half),
                        }),
                )
            })
    }

    /// The view header: back and forward, the note's folder and name, the
    /// graph toggle and the mode toggle. The buttons act on their own
    /// group, which they make active first.
    fn render_header(&self, view: &GroupView, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let id = view.id;
        // The leaf's name, the same string its tab carries: "New tab" with
        // nothing open, not "No file is open".
        let title = leaf_title(view.current.as_deref(), view.mode);
        let crumbs: SharedString = view
            .current
            .as_deref()
            .and_then(|p| p.rsplit_once('/').map(|(dir, _)| dir.replace('/', " / ")))
            .unwrap_or_default()
            .into();
        // Obsidian's view action shows the current mode: a book while
        // reading, a pencil while editing.
        let mode_icon = match view.mode {
            ViewMode::Reading => "book-open",
            _ => "pencil",
        };
        let can_back = view.history_ix > 1;
        let can_forward = view.history_ix < view.history_len;
        let title_color = if view.active {
            theme.text
        } else {
            theme.text_muted
        };
        // Obsidian's view header: 40 px, 12 px side padding, the history
        // arrows at 40 percent while they have nowhere to go, the title in
        // the small UI size, the actions as icons at the right.
        div()
            .flex()
            .flex_row()
            .items_center()
            .h(px(HEADER_HEIGHT))
            .px(px(12.))
            .gap(px(8.))
            .text_size(px(FONT_UI_SMALL))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .child(
                        icon_button("back", "arrow-left", theme)
                            .when(!can_back, |d| d.opacity(0.4))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.activate_group(id, cx);
                                this.go_back(&GoBack, window, cx)
                            })),
                    )
                    .child(
                        icon_button("forward", "arrow-right", theme)
                            .when(!can_forward, |d| d.opacity(0.4))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.activate_group(id, cx);
                                this.go_forward(&GoForward, window, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .gap_1()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .when(!crumbs.is_empty(), |d| {
                        d.child(div().text_color(theme.text_muted).child(crumbs.clone()))
                            .child(div().text_color(theme.text_muted).child("/"))
                    })
                    .child(self.render_title(view, title.clone(), title_color, theme, cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(2.))
                    .child(
                        icon_button("graph", "git-fork", theme)
                            .when(view.mode == ViewMode::Graph, |d| d.bg(theme.bg_active))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.activate_group(id, cx);
                                this.open_graph_action(&OpenGraph, window, cx)
                            })),
                    )
                    .when(header_shows_mode_switch(&view.current, view.mode), |d| {
                        d.child(icon_button("mode", mode_icon, theme).on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.activate_group(id, cx);
                                this.toggle_mode(&ToggleMode, window, cx)
                            },
                        )))
                    }),
            )
    }

    /// The view header's title, Obsidian's `.view-header-title`: a field
    /// that is editable where it stands, so a single click starts an edit
    /// (see [[Specification#The view-header title is an inline rename
    /// field]]).
    ///
    /// The text is painted as a shaped line rather than put in a div, for
    /// two reasons: a click has to be measured against the glyphs to land
    /// the caret where the pointer was, and an open edit draws its caret
    /// and selection in the same place. A leaf with no file is not
    /// renamed, so "New tab" is drawn and clicks on it do nothing.
    fn render_title(
        &self,
        view: &GroupView,
        title: SharedString,
        color: Hsla,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let id = view.id;
        let editing = self.title_edit.as_ref().is_some_and(|e| e.group == id);
        let read_entity = cx.entity();
        let paint_entity = cx.entity();
        let read_focus = self.title_focus.clone();
        let paint_focus = self.title_focus.clone();
        let mut selection_color = theme.accent;
        selection_color.a = if theme.dark { 0.35 } else { 0.25 };
        let caret_color = theme.text;
        let body = canvas(
            move |bounds, window, cx| {
                let style = window.text_style();
                let font_size = style.font_size.to_pixels(window.rem_size());
                let line_height = window.line_height();
                let ws = read_entity.read(cx);
                let edit = ws.title_edit.as_ref().filter(|e| e.group == id);
                let text: SharedString = match edit {
                    Some(edit) => edit.field.text().to_string().into(),
                    None => title.clone(),
                };
                let cursor = edit.map(|e| e.field.cursor()).unwrap_or(0);
                let selection = edit.map(|e| e.field.selection()).filter(|s| !s.is_empty());
                let mut run = style.to_run(text.len());
                run.color = color;
                let line = window
                    .text_system()
                    .shape_line(text.clone(), font_size, &[run], None);
                let cursor_x = if text.is_empty() {
                    px(0.)
                } else {
                    line.x_for_index(cursor)
                };
                let selection_x =
                    selection.map(|s| line.x_for_index(s.start)..line.x_for_index(s.end));
                // A title too long for the header scrolls to keep the
                // caret in view, the way the crate's other field does.
                let width = bounds.size.width;
                let mut scroll_x = ws
                    .title_layout
                    .get(&id)
                    .map(|l| l.scroll_x)
                    .unwrap_or(px(0.));
                if cursor_x - scroll_x > width - px(2.) {
                    scroll_x = cursor_x - width + px(2.);
                }
                if cursor_x - scroll_x < px(0.) {
                    scroll_x = cursor_x;
                }
                if line.width < width {
                    scroll_x = px(0.);
                }
                TitlePrepaint {
                    line,
                    cursor_x,
                    selection_x,
                    scroll_x,
                    line_height,
                    editing: edit.is_some(),
                    caret: edit.is_some() && read_focus.is_focused(window),
                }
            },
            move |bounds, prepaint: TitlePrepaint, window, cx| {
                // The platform's input method is pointed at the field only
                // while one is open: the title is not a tab stop, and
                // nothing types into it until a click puts a caret there.
                if prepaint.editing {
                    window.handle_input(
                        &paint_focus,
                        ElementInputHandler::new(bounds, paint_entity.clone()),
                        cx,
                    );
                }
                let y = bounds.origin.y + (bounds.size.height - prepaint.line_height) / 2.;
                let origin = point(bounds.origin.x - prepaint.scroll_x, y);
                window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
                    if let Some(selection) = &prepaint.selection_x {
                        window.paint_quad(fill(
                            Bounds::from_corners(
                                point(origin.x + selection.start, y),
                                point(origin.x + selection.end, y + prepaint.line_height),
                            ),
                            selection_color,
                        ));
                    }
                    let _ = prepaint.line.paint(
                        origin,
                        prepaint.line_height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                    if prepaint.caret {
                        window.paint_quad(fill(
                            Bounds::new(
                                point(origin.x + prepaint.cursor_x, y + px(2.)),
                                size(px(1.5), prepaint.line_height - px(4.)),
                            ),
                            caret_color,
                        ));
                    }
                });
                paint_entity.update(cx, |ws, _| {
                    ws.title_layout.insert(
                        id,
                        TitleLayout {
                            bounds,
                            line: prepaint.line,
                            line_height: prepaint.line_height,
                            scroll_x: prepaint.scroll_x,
                        },
                    );
                });
            },
        )
        .size_full();
        let mut host = div()
            .id(("view-title", id))
            .flex_1()
            .min_w_0()
            // The canvas has no size of its own, so the row's height is
            // given here; the line is centred in it when it paints.
            .h(px(HEADER_HEIGHT))
            .flex()
            .items_center()
            .overflow_hidden()
            .text_size(px(FONT_UI_SMALL))
            .text_color(color)
            .child(body);
        if let Some(path) = view.current.clone() {
            host = host.cursor_text().on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.title_clicked(id, path.clone(), event, window, cx)
                }),
            );
        }
        if !editing {
            return host.into_any_element();
        }
        // The keys only come here while a field is open, and they are the
        // crate's own field keys: the same context, so the same bindings.
        host.track_focus(&self.title_focus)
            .key_context("TextInput")
            .on_action(cx.listener(|this, _: &text_input::Left, _, cx| {
                this.with_title_field(cx, |f| f.step(false, false))
            }))
            .on_action(cx.listener(|this, _: &text_input::Right, _, cx| {
                this.with_title_field(cx, |f| f.step(true, false))
            }))
            .on_action(cx.listener(|this, _: &text_input::SelectLeft, _, cx| {
                this.with_title_field(cx, |f| f.step(false, true))
            }))
            .on_action(cx.listener(|this, _: &text_input::SelectRight, _, cx| {
                this.with_title_field(cx, |f| f.step(true, true))
            }))
            .on_action(cx.listener(|this, _: &text_input::WordLeft, _, cx| {
                this.with_title_field(cx, |f| f.step_word(false, false))
            }))
            .on_action(cx.listener(|this, _: &text_input::WordRight, _, cx| {
                this.with_title_field(cx, |f| f.step_word(true, false))
            }))
            .on_action(cx.listener(|this, _: &text_input::SelectWordLeft, _, cx| {
                this.with_title_field(cx, |f| f.step_word(false, true))
            }))
            .on_action(cx.listener(|this, _: &text_input::SelectWordRight, _, cx| {
                this.with_title_field(cx, |f| f.step_word(true, true))
            }))
            .on_action(cx.listener(|this, _: &text_input::Home, _, cx| {
                this.with_title_field(cx, |f| f.move_to_edge(false, false))
            }))
            .on_action(cx.listener(|this, _: &text_input::End, _, cx| {
                this.with_title_field(cx, |f| f.move_to_edge(true, false))
            }))
            .on_action(cx.listener(|this, _: &text_input::SelectHome, _, cx| {
                this.with_title_field(cx, |f| f.move_to_edge(false, true))
            }))
            .on_action(cx.listener(|this, _: &text_input::SelectEnd, _, cx| {
                this.with_title_field(cx, |f| f.move_to_edge(true, true))
            }))
            .on_action(cx.listener(|this, _: &text_input::SelectAll, _, cx| {
                this.with_title_field(cx, |f| f.select_all())
            }))
            .on_action(cx.listener(|this, _: &text_input::Backspace, _, cx| {
                this.with_title_field(cx, |f| f.backspace())
            }))
            .on_action(cx.listener(|this, _: &text_input::Delete, _, cx| {
                this.with_title_field(cx, |f| f.delete())
            }))
            .on_action(
                cx.listener(|this, _: &text_input::DeleteWordBackward, _, cx| {
                    this.with_title_field(cx, |f| f.delete_word_back())
                }),
            )
            .on_action(cx.listener(|this, _: &text_input::Clear, _, cx| {
                this.with_title_field(cx, |f| {
                    f.select_all();
                    f.insert("");
                })
            }))
            .on_action(cx.listener(|this, _: &text_input::Copy, _, cx| this.title_copy(false, cx)))
            .on_action(cx.listener(|this, _: &text_input::Cut, _, cx| this.title_copy(true, cx)))
            .on_action(cx.listener(|this, _: &text_input::Paste, _, cx| this.title_paste(cx)))
            .on_action(
                cx.listener(|this, _: &text_input::Submit, window, cx| {
                    this.commit_title(window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &text_input::Cancel, window, cx| {
                    this.cancel_title(window, cx)
                }),
            )
            // Ctrl+E in a field is swallowed rather than toggling the mode
            // behind it, as it is in every other field here.
            .on_action(cx.listener(|_, _: &text_input::Swallow, _, _| {}))
            .into_any_element()
    }

    /// A click on a view-header title: Obsidian's focuses the field and
    /// puts the caret at the offset the pointer landed on, and a double
    /// click takes the word it landed in -- no modal and no menu at any
    /// point. A click past the end of the text is a click on the header,
    /// not on the title, and changes nothing.
    fn title_clicked(
        &mut self,
        group: u64,
        path: String,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(layout) = self.title_layout.get(&group).cloned() else {
            return;
        };
        let x = event.position.x - layout.bounds.origin.x + layout.scroll_x;
        if x > layout.line.width + px(4.) {
            return;
        }
        // A click in another group's header makes that group active first,
        // so the edit always belongs to the leaf the keyboard is in.
        self.activate_group(group, cx);
        // The field holds the name the header shows, which is the name
        // without its extension -- the same stem the rename prompt's box
        // holds, so both commit the same way.
        let name = leaf_title(Some(&path), ViewMode::Reading).to_string();
        let mut edit = match self.title_edit.take() {
            Some(edit) if edit.group == group && edit.path == path => edit,
            _ => TitleEdit {
                group,
                path,
                field: TitleField::new(name),
                marked: None,
            },
        };
        let mut at = layout
            .line
            .closest_index_for_x(x)
            .min(edit.field.text().len());
        while !edit.field.text().is_char_boundary(at) {
            at -= 1;
        }
        edit.field
            .click(at, event.click_count, event.modifiers.shift);
        self.title_edit = Some(edit);
        // The field's focus handle only joins the dispatch tree with the
        // frame that paints it, and the click that opens one lands before
        // that frame: a field that was not there yet takes the keyboard on
        // the next frame instead.
        let focus = self.title_focus.clone();
        window.focus(&focus, cx);
        window.on_next_frame(move |window, cx| window.focus(&focus, cx));
        cx.notify();
    }

    /// Applies an editing key to the open title field.
    fn with_title_field(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut TitleField)) {
        if let Some(edit) = self.title_edit.as_mut() {
            f(&mut edit.field);
            edit.marked = None;
            cx.notify();
        }
    }

    fn title_copy(&mut self, cut: bool, cx: &mut Context<Self>) {
        let Some(edit) = self.title_edit.as_mut() else {
            return;
        };
        let range = edit.field.selection();
        if range.is_empty() {
            return;
        }
        let text = edit.field.text()[range.clone()].to_string();
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        if cut {
            edit.field.replace(range, "");
            edit.marked = None;
            cx.notify();
        }
    }

    fn title_paste(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        self.with_title_field(cx, |f| f.insert(&text));
    }

    /// Enter in the view-header title: the file takes the typed name. The
    /// field holds the stem, so this goes through the same path the rename
    /// prompt's box does -- one validator, one rename, one link rewrite.
    /// A name the filesystem would refuse leaves the field open and says
    /// why, rather than losing what was typed.
    fn commit_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.title_edit.take() else {
            return;
        };
        let value = edit.field.text().trim().to_string();
        let to = match self.renamed_path(&edit.path, &value) {
            Ok(to) => to,
            Err(err) => {
                self.title_edit = Some(edit);
                self.report(err, cx);
                return;
            }
        };
        // The keyboard goes back to the view before the rename, which may
        // put the "Update links?" prompt up and want it for itself.
        self.focus_main(window, cx);
        self.rename_with_links(&edit.path, &to, window, cx);
        cx.notify();
    }

    /// Escape in the view-header title: the edit is abandoned and nothing
    /// is renamed, as Obsidian's does.
    fn cancel_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.title_edit.take().is_none() {
            return;
        }
        self.focus_main(window, cx);
        cx.notify();
    }

    /// The file group `id` shows, wherever that group's fields live.
    fn group_current(&self, id: u64) -> Option<&str> {
        let ix = self.group_ix(id)?;
        if ix == self.active_group {
            return self.current.as_deref();
        }
        let group = &self.groups[ix];
        group.tabs.get(group.active_tab)?.path.as_deref()
    }

    /// [`renamed_path`] for a vault path, which is asked whether it is a
    /// folder.
    fn renamed_path(&self, from: &str, value: &str) -> Result<String, String> {
        let is_folder = self
            .state
            .as_ref()
            .map(|s| s.vault().absolute(from).is_dir())
            .unwrap_or(false);
        renamed_path(from, value, is_folder)
    }

    /// What a group's area paints, by the same rule as [`Workspace::main_view`]
    /// over that group's own views. There is one graph view; a second group
    /// showing the graph says where it is instead.
    fn render_group_main(
        &self,
        view: &GroupView,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let has_note = view.reading.read(cx).path.is_some();
        let image = view
            .current
            .as_deref()
            .is_some_and(|p| FileKind::from_path(p) == FileKind::Image);
        let has_buffer = !view.editor.read(cx).name.is_empty();
        let canvas =
            view.current.is_some() && view.canvas.read(cx).path() == view.current.as_deref();
        let bases = view.current.is_some() && view.bases.read(cx).path() == view.current.as_deref();
        let pdf = view.current.is_some() && view.pdf.read(cx).path() == view.current.as_deref();
        match Self::main_view_for(view.mode, has_note, image, has_buffer, canvas, bases, pdf) {
            MainView::Canvas => view.canvas.clone().into_any_element(),
            MainView::Bases => view.bases.clone().into_any_element(),
            MainView::Pdf => view.pdf.clone().into_any_element(),
            MainView::Graph if self.graph_group() == Some(view.id) => {
                self.graph.clone().into_any_element()
            }
            MainView::Graph => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.text_muted)
                .child("The graph is open in another pane")
                .into_any_element(),
            MainView::Reading => view.reading.clone().into_any_element(),
            MainView::Editor => Self::render_editor(&view.editor, view.detached.clone(), theme),
            MainView::Empty => match view.current.as_deref() {
                Some(path) => Self::render_unsupported_file(path, theme),
                None => self.render_empty_tab(theme, cx),
            },
        }
    }

    /// The editor, under a line naming the file it lost if its buffer is no
    /// longer backed by one. The user has to be told; the alternative is
    /// typing into a buffer that is never written and never mentioned.
    fn render_editor(
        editor: &Entity<Editor>,
        detached: Option<SharedString>,
        theme: &Theme,
    ) -> gpui::AnyElement {
        let Some(name) = detached else {
            return editor.clone().into_any_element();
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .px_3()
                    .py_1()
                    .text_size(px(12.))
                    .bg(theme.bg_alt)
                    .text_color(theme.text_muted)
                    .border_b_1()
                    .border_color(theme.border)
                    .child(format!(
                        "{name} — this buffer no longer belongs to a file in the vault, \
                         and nothing saves it"
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .child(editor.clone().into_any_element()),
            )
            .into_any_element()
    }

    /// A file whose kind no view can draw: an error message, not a front
    /// door, so it gets the line on its own.
    fn render_unsupported_file(path: &str, theme: &Theme) -> gpui::AnyElement {
        let message: SharedString = format!(
            "{:?} files are not supported yet",
            FileKind::from_path(path)
        )
        .into();
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_5()
            .items_center()
            .justify_center()
            .text_color(theme.text_muted)
            .child(message)
            .into_any_element()
    }

    /// The empty tab: the four pills Obsidian offers there, each running
    /// the command it names, under Silica's mark -- an empty tab is the one
    /// blank canvas the window has, so it is where the mark goes. The body
    /// is the same however the tab came to be empty, a fresh vault and the
    /// last tab closing alike, because both leave a tab with no file.
    fn render_empty_tab(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let registry = cx.global::<Commands>().0.clone();
        let pills: Vec<_> =
            empty_tab_actions(&registry)
                .into_iter()
                .map(|action| {
                    let command = action.command;
                    // Obsidian's `mod-pill`: fully rounded, the label in the
                    // UI size with the hotkey behind it at muted strength.
                    div()
                        .id(action.id)
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.))
                        .px(px(14.))
                        .py(px(6.))
                        .rounded(px(14.))
                        .bg(theme.bg_alt)
                        .border_1()
                        .border_color(theme.border)
                        .cursor_pointer()
                        .hover(|s| s.bg(theme.bg_active))
                        .text_size(px(FONT_UI_SMALL))
                        .child(div().text_color(theme.text).child(action.label))
                        .children(
                            action
                                .hotkey
                                .map(|keys| div().text_color(theme.text_faint).child(keys)),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.run_command(command, window, cx)
                        }))
                })
                .collect();
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_5()
            .items_center()
            .justify_center()
            .text_color(theme.text_muted)
            .child(logo(104.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .justify_center()
                    .gap(px(8.))
                    .children(pills),
            )
            .into_any_element()
    }

    // ----- tabs ------------------------------------------------------------------

    /// Copies the active tab's live fields into its slot.
    fn stash_active_tab(&mut self) {
        let slot = self.tabs.get(self.active_tab);
        let leaf = slot.and_then(|t| t.leaf.clone());
        // The view state belongs to the canvas the tab shows; a tab that
        // moved on to another file leaves it behind.
        let view_state = slot
            .filter(|t| t.path == self.current)
            .and_then(|t| t.view_state.clone());
        let tab = Tab {
            path: self.current.clone(),
            mode: self.mode,
            history: self.history.clone(),
            history_ix: self.history_ix,
            leaf,
            view_state,
        };
        match self.tabs.get_mut(self.active_tab) {
            Some(slot) => *slot = tab,
            None => {
                self.tabs.push(tab);
                self.active_tab = self.tabs.len() - 1;
            }
        }
    }

    /// Every tab as it stands, the active one from the live fields.
    fn tab_snapshots(&self) -> Vec<Tab> {
        let mut tabs = self.tabs.clone();
        if let Some(slot) = tabs.get_mut(self.active_tab) {
            slot.path = self.current.clone();
            slot.mode = self.mode;
            slot.history = self.history.clone();
            slot.history_ix = self.history_ix;
        }
        tabs
    }

    /// Puts `tabs[active_tab]` on screen: its note in its mode, or the
    /// empty state.
    fn load_tab(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active_tab).cloned() else {
            return;
        };
        self.history = tab.history;
        self.history_ix = tab.history_ix;
        if tab.mode == ViewMode::Graph && self.mode != ViewMode::Graph {
            self.mode_before_graph = self.mode;
        }
        self.mode = tab.mode;
        if tab.mode.is_editing() {
            self.editor.update(cx, |e, cx| {
                e.set_live_preview(tab.mode == ViewMode::LivePreview, cx)
            });
        }
        match (tab.path, self.state.clone()) {
            (Some(path), Some(state)) => {
                self.current = None;
                self.show_file(&path, &state, cx);
            }
            (None, Some(state)) => self.show_empty(&state, cx),
            (path, None) => self.current = path,
        }
        self.request_focus_main(cx);
        cx.notify();
    }

    /// The empty tab: no note in the main area and nothing describing one.
    /// A buffer with unsaved edits nowhere else stays, marked as saved
    /// nowhere, the way a file that went away leaves it.
    fn show_empty(&mut self, state: &Arc<VaultState>, cx: &mut Context<Self>) {
        self.release_buffer(cx);
        self.release_canvas(cx);
        self.release_bases(cx);
        self.release_pdf(cx);
        self.reading.update(cx, |r, cx| {
            r.set_note(
                None,
                reading::build_with("", "", state.as_ref(), state.build_options()),
                cx,
            )
        });
        self.current = None;
        self.explorer.update(cx, |e, cx| e.clear_active(cx));
        self.graph.update(cx, |g, cx| g.set_current(None, cx));
        self.clear_panes(cx);
        self.status = SharedString::default();
    }

    /// `workspace:new-tab`: an empty tab to the right of the active one,
    /// in the mode "Default view for new tabs" names.
    /// `workspace:new-tab` and the strip's `+`: the new tab goes on the
    /// strip's right end, whichever tab was active, and becomes the active
    /// one (probed 2026-10-22, `tools/obsidian-probe/new-tab.mjs`).
    pub fn new_tab(&mut self, cx: &mut Context<Self>) {
        self.flush_save(cx);
        self.stash_active_tab();
        let mode = self.default_mode();
        self.tabs.push(Tab::empty(mode));
        self.active_tab = self.tabs.len() - 1;
        self.load_tab(cx);
    }

    /// Opens `path` in a new tab, leaving the active one as it is.
    pub fn open_in_new_tab(
        &mut self,
        path: String,
        subpath: Option<Subpath<'_>>,
        cx: &mut Context<Self>,
    ) {
        self.new_tab(cx);
        self.open_file(path, subpath, cx);
    }

    pub fn activate_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() || ix == self.active_tab {
            return;
        }
        self.flush_save(cx);
        self.stash_active_tab();
        self.active_tab = ix;
        self.load_tab(cx);
    }

    /// `workspace:next-tab` and `previous-tab`: `step` tabs along, wrapping.
    fn cycle_tab(&mut self, step: usize, cx: &mut Context<Self>) {
        let n = self.tabs.len();
        if n > 1 {
            self.activate_tab((self.active_tab + step) % n, cx);
        }
    }

    /// `workspace:close`: the last close leaves the empty tab, as Obsidian's
    /// does; the tab to the right takes over, or the one to the left at the
    /// end of the strip.
    pub fn close_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.flush_save(cx);
        self.stash_active_tab();
        if ix >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(ix);
        self.remember_closed(ix, tab);
        if self.tabs.is_empty() {
            // A group whose last tab closes goes with it, as Obsidian's
            // does, unless it is the only one.
            if self.groups.len() > 1 {
                self.tabs.push(Tab::empty(self.default_mode()));
                self.active_tab = 0;
                let id = self.active_group_id();
                self.close_group(id, false, cx);
                return;
            }
            self.tabs.push(Tab::empty(self.default_mode()));
            self.active_tab = 0;
            self.load_tab(cx);
        } else if ix == self.active_tab {
            self.active_tab = ix.min(self.tabs.len() - 1);
            self.load_tab(cx);
        } else {
            if ix < self.active_tab {
                self.active_tab -= 1;
            }
            cx.notify();
        }
    }

    fn remember_closed(&mut self, ix: usize, tab: Tab) {
        // An empty tab is not worth bringing back.
        if tab.path.is_some() || tab.mode == ViewMode::Graph {
            self.closed_tabs.push((ix, tab));
            if self.closed_tabs.len() > CLOSED_TABS_KEPT {
                self.closed_tabs.remove(0);
            }
        }
    }

    /// `workspace:close-others`: every tab but the active one.
    fn close_other_tabs(&mut self, cx: &mut Context<Self>) {
        self.stash_active_tab();
        let keep = self.tabs.remove(self.active_tab);
        let others = std::mem::take(&mut self.tabs);
        for (ix, tab) in others.into_iter().enumerate() {
            self.remember_closed(ix, tab);
        }
        self.tabs.push(keep);
        self.active_tab = 0;
        cx.notify();
    }

    /// `workspace:undo-close-pane`: the most recently closed tab, back
    /// where it was.
    fn undo_close_tab(&mut self, cx: &mut Context<Self>) {
        let Some((ix, tab)) = self.closed_tabs.pop() else {
            return;
        };
        self.flush_save(cx);
        self.stash_active_tab();
        let at = ix.min(self.tabs.len());
        self.tabs.insert(at, tab);
        self.active_tab = at;
        self.load_tab(cx);
    }

    // ----- tab groups --------------------------------------------------------------

    fn active_group_id(&self) -> u64 {
        self.groups[self.active_group].id
    }

    fn group_ix(&self, id: u64) -> Option<usize> {
        self.groups.iter().position(|g| g.id == id)
    }

    /// Copies the active group's live fields into its slot.
    fn stash_active_group(&mut self) {
        self.stash_active_tab();
        let ix = self.active_group;
        let group = &mut self.groups[ix];
        group.tabs = self.tabs.clone();
        group.active_tab = self.active_tab;
        group.mode_before_graph = self.mode_before_graph;
        group.detached = self.detached.clone();
        group.words = self.words;
        group.chars = self.chars;
    }

    /// Makes group `ix` the live one. Nothing is reloaded — its views still
    /// show its note — but everything that follows the active note (the
    /// explorer's highlight, the graph's centre, the right dock, the status
    /// bar) is pointed at it, and a note that went away while the group was
    /// off screen is let go of the way the watcher would have.
    fn load_group(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.active_group = ix;
        let group = &self.groups[ix];
        let id = group.id;
        self.tabs = group.tabs.clone();
        self.active_tab = group.active_tab.min(self.tabs.len().saturating_sub(1));
        self.mode_before_graph = group.mode_before_graph;
        self.detached = group.detached.clone();
        self.words = group.words;
        self.chars = group.chars;
        self.editor = group.editor.clone();
        self.reading = group.reading.clone();
        self.canvas = group.canvas.clone();
        self.bases = group.bases.clone();
        self.pdf = group.pdf.clone();
        let tab = self
            .tabs
            .get(self.active_tab)
            .cloned()
            .unwrap_or_else(|| Tab::empty(self.default_mode()));
        self.current = tab.path;
        self.mode = tab.mode;
        self.history = tab.history;
        self.history_ix = tab.history_ix;
        self.group_mru.retain(|g| *g != id);
        self.group_mru.insert(0, id);
        self.save_task = None;
        match (self.current.clone(), self.state.clone()) {
            (Some(path), Some(state)) => {
                if state.vault().absolute(&path).is_file() {
                    self.explorer.update(cx, |e, cx| e.follow(&path, cx));
                    self.graph
                        .update(cx, |g, cx| g.set_current(Some(path.clone()), cx));
                    self.refresh_panes(&state, cx);
                    self.status = path.into();
                } else {
                    self.detach_current(true, &state, cx);
                }
            }
            (None, Some(state)) => self.show_empty(&state, cx),
            (_, None) => {}
        }
        self.request_focus_main(cx);
        cx.notify();
    }

    /// Hands the keyboard and the live fields to group `id`: a click in its
    /// area, a focus command, or the group that takes over when one closes.
    pub fn activate_group(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(ix) = self.group_ix(id) else {
            return;
        };
        if ix == self.active_group {
            return;
        }
        self.flush_save(cx);
        self.stash_active_group();
        self.load_group(ix, cx);
    }

    /// A group of its own, with an editor and a reading view set up like the
    /// others', not yet in the layout.
    fn new_group(&mut self, mode: ViewMode, window: &mut Window, cx: &mut Context<Self>) -> u64 {
        let editor = cx.new(|cx| Editor::new(window, cx));
        let reading = cx.new(ReadingView::new);
        let canvas = cx.new(CanvasView::new);
        let bases = cx.new(BasesView::new);
        let pdf = cx.new(PdfView::new);
        self._subscriptions.extend(view_subscriptions(
            &editor, &reading, &canvas, &bases, &pdf, cx,
        ));
        let id = self.next_group_id;
        self.next_group_id += 1;
        if let Some(state) = self.state.clone() {
            Self::give_editor_the_vault(&editor, &state, cx);
            Self::give_reading_the_vault(&reading, &state, cx);
        }
        self.groups
            .push(TabGroup::new(id, editor, reading, canvas, bases, pdf, mode));
        if let Some(state) = self.state.clone() {
            self.apply_app_config(&state, cx);
        }
        id
    }

    /// `workspace:split-vertical` ("Split right") and `split-horizontal`
    /// ("Split down"): a new group beside the active one, showing the same
    /// note in the same mode, and it takes the focus. A `dimension` the group
    /// had is halved between the two, or moves to the split that now wraps
    /// them (see [[Specification#Observed tab group behavior]]).
    fn split(&mut self, direction: SplitDirection, window: &mut Window, cx: &mut Context<Self>) {
        self.split_beside(None, direction, window, cx);
    }

    /// The explorer menu's "Open to the right": a new group beside the active
    /// one showing `path`, which is [`Workspace::split`] with a file of its
    /// own. The group it leaves behind keeps whatever it was showing, which is
    /// the difference from a split.
    fn open_to_right(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        self.split_beside(Some(path), SplitDirection::Vertical, window, cx);
    }

    /// What the two share. `None` duplicates the active tab -- the same note,
    /// mode, history and view state, which is what a split is; `Some(path)` is
    /// a fresh tab on that file, with no history and no view state to inherit,
    /// opened in the vault's default view mode the way any first open is.
    ///
    /// The order here matters and is the order [`Workspace::split`] had: the
    /// save is flushed and the group stashed *before* the tab is built, so the
    /// `view_state` it copies is the one the stash just wrote.
    fn split_beside(
        &mut self,
        path: Option<String>,
        direction: SplitDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.flush_save(cx);
        self.stash_active_group();
        let from = self.active_group_id();
        let (tab, mode_before_graph) = match path {
            None => (
                Tab {
                    path: self.current.clone(),
                    mode: self.mode,
                    history: self.history.clone(),
                    history_ix: self.history_ix,
                    leaf: None,
                    view_state: self
                        .tabs
                        .get(self.active_tab)
                        .and_then(|t| t.view_state.clone()),
                },
                self.mode_before_graph,
            ),
            Some(path) => {
                let mode = self.default_mode();
                (
                    Tab {
                        path: Some(path),
                        mode,
                        history: Vec::new(),
                        history_ix: 0,
                        leaf: None,
                        view_state: None,
                    },
                    mode,
                )
            }
        };
        let id = self.new_group(tab.mode, window, cx);
        self.place_beside(from, id, direction, true);
        let Some(ix) = self.group_ix(id) else {
            return;
        };
        self.groups[ix].tabs = vec![tab];
        self.groups[ix].active_tab = 0;
        self.groups[ix].mode_before_graph = mode_before_graph;
        self.load_group(ix, cx);
        // The new group's views are empty: put the note in them.
        if let (Some(path), Some(state)) = (self.current.clone(), self.state.clone()) {
            if self.mode.is_editing() {
                let live = self.mode == ViewMode::LivePreview;
                self.editor.update(cx, |e, cx| e.set_live_preview(live, cx));
            }
            self.show_file(&path, &state, cx);
        }
    }

    /// Puts group `new` into the layout beside group `from`, after it or
    /// before it along `direction`, and settles the `dimension`: halved
    /// between the two when they became siblings, moved onto the split that
    /// now wraps them when they did not.
    fn place_beside(&mut self, from: u64, new: u64, direction: SplitDirection, after: bool) {
        let Some(from_ix) = self.group_ix(from) else {
            return;
        };
        let dimension = self.groups[from_ix].dimension();
        match self.layout_tree.insert_beside(from, new, direction, after) {
            Some(false) => {
                if let Some(d) = dimension {
                    self.groups[from_ix].set_dimension(Some(d / 2.0));
                    if let Some(new_ix) = self.group_ix(new) {
                        self.groups[new_ix].set_dimension(Some(d / 2.0));
                    }
                }
            }
            Some(true) => {
                self.groups[from_ix].set_dimension(None);
                if let Some(mut path) = self.layout_tree.path_to(from) {
                    path.pop();
                    if let Some(Layout::Split { node, .. }) = self.layout_tree.node_at_mut(&path) {
                        set_dimension(node, dimension);
                    }
                }
            }
            None => {}
        }
    }

    // ----- dragging tabs ---------------------------------------------------------

    /// A tab drag started: every group paints its drop zones from the next
    /// frame on. Called from the drag's own constructor, which runs with
    /// the app rather than this view, so it comes in through a weak handle.
    fn begin_tab_drag(&mut self, cx: &mut Context<Self>) {
        self.dragging_tab = true;
        self.tab_drop = None;
        self.drop_bar = None;
        self.tab_bounds.clear();
        cx.notify();
    }

    /// The dragged tab moved over (or off) group `group`'s strip: over it,
    /// the drop bar goes to the tab boundary nearest the pointer, the left
    /// edge of a tab or the right edge of the last one, sliding there from
    /// wherever it was. Off it, the bar goes if it was this strip's.
    fn strip_drag_moved(
        &mut self,
        group: u64,
        bounds: gpui::Bounds<gpui::Pixels>,
        position: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        if !bounds.contains(&position) {
            if self.drop_bar.is_some_and(|b| b.group == group) {
                self.drop_bar = None;
                cx.notify();
            }
            return;
        }
        let mut tabs: Vec<(usize, gpui::Bounds<gpui::Pixels>)> = self
            .tab_bounds
            .iter()
            .filter(|((g, _), _)| *g == group)
            .map(|((_, ix), b)| (*ix, *b))
            .collect();
        tabs.sort_by_key(|(ix, _)| *ix);
        let Some((last_ix, last)) = tabs.last().copied() else {
            return;
        };
        let pointer = f32::from(position.x);
        let edges = tabs
            .iter()
            .map(|(ix, b)| (*ix, f32::from(b.origin.x)))
            .chain(std::iter::once((last_ix + 1, f32::from(last.right()))));
        let Some((at, x)) =
            edges.min_by(|a, b| (a.1 - pointer).abs().total_cmp(&(b.1 - pointer).abs()))
        else {
            return;
        };
        let now = Instant::now();
        let strip_x = f32::from(bounds.origin.x);
        let next = match self.drop_bar {
            Some(bar) if bar.group == group && bar.at == at => {
                if bar.strip_x == strip_x {
                    return;
                }
                DropBar { strip_x, ..bar }
            }
            // Within the strip the bar slides; arriving from elsewhere it
            // appears where it is wanted.
            Some(bar) if bar.group == group => DropBar {
                group,
                at,
                x,
                from: bar.shown_x(now),
                since: now,
                strip_x,
            },
            _ => DropBar {
                group,
                at,
                x,
                from: x,
                since: now,
                strip_x,
            },
        };
        self.drop_bar = Some(next);
        cx.notify();
    }

    /// A tab dropped on group `to`'s strip lands where the drop bar says,
    /// or at `fallback` when the bar never found a boundary.
    fn drop_tab_at_bar(
        &mut self,
        drag: &TabDrag,
        to: u64,
        fallback: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        let at = match self.drop_bar {
            Some(bar) if bar.group == to => Some(bar.at),
            _ => fallback,
        };
        self.drop_tab_on_strip(drag, to, at, cx);
    }

    /// The dragged tab moved: over group `id`'s content at `position` it
    /// lights the zone it would land in, and leaving it puts the light out.
    /// Every group hears every move, so only the one under the pointer
    /// writes, and only the one that was lit clears.
    fn tab_drag_moved(
        &mut self,
        id: u64,
        bounds: gpui::Bounds<gpui::Pixels>,
        position: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        let next = if bounds.contains(&position) {
            Some((id, DropZone::at(bounds, position)))
        } else if self.tab_drop.is_some_and(|(g, _)| g == id) {
            None
        } else {
            return;
        };
        if self.tab_drop != next {
            self.tab_drop = next;
            cx.notify();
        }
    }

    /// A tab dropped on group `id`'s content: into its strip from the
    /// middle, into a new group beside it from a side.
    fn drop_tab_on_group(
        &mut self,
        drag: &TabDrag,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let zone = match self.tab_drop.take() {
            Some((g, zone)) if g == id => zone,
            _ => DropZone::Center,
        };
        self.dragging_tab = false;
        self.drop_bar = None;
        self.tab_bounds.clear();
        match zone.split() {
            None => self.move_tab(drag, id, None, cx),
            Some((direction, after)) => self.split_with_tab(drag, id, direction, after, window, cx),
        }
    }

    /// A tab dropped on group `to`'s strip, before tab `at` or at the end:
    /// it joins that strip, there, and is the active tab.
    fn drop_tab_on_strip(
        &mut self,
        drag: &TabDrag,
        to: u64,
        at: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        self.tab_drop = None;
        self.dragging_tab = false;
        self.drop_bar = None;
        self.tab_bounds.clear();
        self.move_tab(drag, to, at, cx);
    }

    /// Takes tab `ix` out of group `from_ix` with every group stashed. A
    /// group whose active tab left shows the one that takes its place,
    /// which means making it live for a moment: its views only ever load
    /// through the live fields. Says whether the group has nothing left.
    fn take_tab(&mut self, from_ix: usize, ix: usize, cx: &mut Context<Self>) -> Option<Tab> {
        let group = &mut self.groups[from_ix];
        if ix >= group.tabs.len() {
            return None;
        }
        let tab = group.tabs.remove(ix);
        if group.tabs.is_empty() {
            return Some(tab);
        }
        if ix < group.active_tab {
            group.active_tab -= 1;
        } else if ix == group.active_tab {
            group.active_tab = ix.min(group.tabs.len() - 1);
            self.load_group(from_ix, cx);
            self.load_tab(cx);
            self.stash_active_group();
        }
        Some(tab)
    }

    /// Moves the dragged tab to group `to`, before its tab `at` or at the
    /// end, and makes it the active tab of the active group. In its own
    /// group that is a reorder. A group left with nothing closes, as one
    /// whose last tab closed does.
    fn move_tab(&mut self, drag: &TabDrag, to: u64, at: Option<usize>, cx: &mut Context<Self>) {
        self.flush_save(cx);
        self.stash_active_group();
        let (Some(from_ix), Some(to_ix)) = (self.group_ix(drag.group), self.group_ix(to)) else {
            return;
        };
        let mut at = at.unwrap_or(usize::MAX);
        let mut empty = false;
        let tab = if from_ix == to_ix {
            // A reorder: the strip's views keep showing what they show
            // until the end, when the dragged tab takes over if it must.
            let group = &mut self.groups[to_ix];
            if drag.tab >= group.tabs.len() {
                return;
            }
            if drag.tab < at {
                at -= 1;
            }
            group.tabs.remove(drag.tab)
        } else {
            let Some(tab) = self.take_tab(from_ix, drag.tab, cx) else {
                return;
            };
            empty = self.groups[from_ix].tabs.is_empty();
            tab
        };
        // What the target's views show now: the tab that was active there,
        // or for a reorder whose active tab left, nothing in particular.
        let target = &mut self.groups[to_ix];
        let shown = (from_ix != to_ix || drag.tab != target.active_tab)
            .then(|| target.tabs.get(target.active_tab).cloned())
            .flatten();
        let at = at.min(target.tabs.len());
        target.tabs.insert(at, tab);
        target.active_tab = at;
        self.load_group(to_ix, cx);
        if shown.is_none() || self.tabs.get(self.active_tab) != shown.as_ref() {
            self.load_tab(cx);
        }
        if empty {
            self.close_group(drag.group, false, cx);
        }
        cx.notify();
    }

    /// Puts the dragged tab in a new group beside group `beside`, after it
    /// or before it along `direction`, the way "Split right" and "Split
    /// down" place theirs, and makes that group active. A group's only tab
    /// dropped on its own edge has nowhere else to be, so nothing happens.
    fn split_with_tab(
        &mut self,
        drag: &TabDrag,
        beside: u64,
        direction: SplitDirection,
        after: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.flush_save(cx);
        self.stash_active_group();
        let Some(from_ix) = self.group_ix(drag.group) else {
            return;
        };
        if drag.group == beside && self.groups[from_ix].tabs.len() <= 1 {
            return;
        }
        if self.group_ix(beside).is_none() {
            return;
        }
        let Some(tab) = self.take_tab(from_ix, drag.tab, cx) else {
            return;
        };
        let empty = self.groups[from_ix].tabs.is_empty();
        let id = self.new_group(tab.mode, window, cx);
        self.place_beside(beside, id, direction, after);
        let Some(ix) = self.group_ix(id) else {
            return;
        };
        self.groups[ix].tabs = vec![tab];
        self.groups[ix].active_tab = 0;
        self.load_group(ix, cx);
        self.load_tab(cx);
        if empty {
            self.close_group(drag.group, false, cx);
        }
        cx.notify();
    }

    /// `workspace:close-tab-group`, and what closing a group's last tab
    /// does: the group goes, its siblings take the room, and the keyboard
    /// goes to the group used most recently before it. The last group
    /// cannot go; it is left with an empty tab instead. `remember` puts the
    /// group's tabs where `workspace:undo-close-pane` finds them.
    fn close_group(&mut self, id: u64, remember: bool, cx: &mut Context<Self>) {
        let Some(ix) = self.group_ix(id) else {
            return;
        };
        if self.groups.len() == 1 {
            self.flush_save(cx);
            self.stash_active_tab();
            let tabs = std::mem::take(&mut self.tabs);
            if remember {
                for (i, tab) in tabs.into_iter().enumerate() {
                    self.remember_closed(i, tab);
                }
            }
            self.tabs.push(Tab::empty(self.default_mode()));
            self.active_tab = 0;
            self.load_tab(cx);
            return;
        }
        let was_active = ix == self.active_group;
        let active_id = self.active_group_id();
        // Nothing paints this group's header again, so its title's shaped
        // line goes with it.
        self.title_layout.remove(&id);
        if was_active {
            self.flush_save(cx);
            self.stash_active_group();
        }
        if remember {
            let tabs = self.groups[ix].tabs.clone();
            for (i, tab) in tabs.into_iter().enumerate() {
                self.remember_closed(i, tab);
            }
        }
        let mut parent = self.layout_tree.path_to(id).unwrap_or_default();
        parent.pop();
        self.layout_tree.remove(id);
        self.groups.remove(ix);
        self.group_mru.retain(|g| *g != id);
        // The split the group left, or the nearest one still standing when
        // it was the split's last child and went with it.
        loop {
            if matches!(
                self.layout_tree.node_at(&parent),
                Some(Layout::Split { .. })
            ) {
                self.renormalise_dimensions(&parent);
                break;
            }
            if parent.pop().is_none() {
                break;
            }
        }
        if was_active {
            let next = self
                .group_mru
                .first()
                .copied()
                .and_then(|g| self.group_ix(g))
                .unwrap_or(0);
            self.load_group(next, cx);
        } else {
            self.active_group = self.group_ix(active_id).unwrap_or(0);
            cx.notify();
        }
    }

    /// The `dimension` of a node of the layout: a group's lives on the
    /// group, a split's on the split.
    fn dimension_of(&self, node: &Layout) -> Option<f64> {
        match node {
            Layout::Group(id) => self.groups[self.group_ix(*id)?].dimension(),
            Layout::Split { node, .. } => node.get("dimension").and_then(Value::as_f64),
        }
    }

    fn set_dimension_at(&mut self, path: &[usize], dimension: Option<f64>) {
        match self.layout_tree.node_at_mut(path) {
            Some(Layout::Group(id)) => {
                let id = *id;
                if let Some(ix) = self.group_ix(id) {
                    self.groups[ix].set_dimension(dimension);
                }
            }
            Some(Layout::Split { node, .. }) => set_dimension(node, dimension),
            None => {}
        }
    }

    /// After a child left the split at `path`, the rest share its room: their
    /// `dimension`s scaled back to a hundred, or dropped when one is left,
    /// which is what Obsidian wrote in both cases.
    fn renormalise_dimensions(&mut self, path: &[usize]) {
        let Some(Layout::Split { children, .. }) = self.layout_tree.node_at(path) else {
            return;
        };
        let dims: Vec<Option<f64>> = children.iter().map(|c| self.dimension_of(c)).collect();
        let n = dims.len();
        let scaled: Vec<Option<f64>> = if n <= 1 {
            vec![None; n]
        } else if dims.iter().all(Option::is_some) {
            let sum: f64 = dims.iter().flatten().sum();
            if sum <= 0.0 {
                return;
            }
            dims.iter().map(|d| d.map(|d| d * 100.0 / sum)).collect()
        } else {
            return;
        };
        for (ix, d) in scaled.into_iter().enumerate() {
            let mut child = path.to_vec();
            child.push(ix);
            self.set_dimension_at(&child, d);
        }
    }

    /// A divider between two children of a split, dragged to `position`
    /// inside the split's `bounds`: the pair's room is redistributed and
    /// every child's share written as a `dimension`, as Obsidian does once
    /// a divider has moved.
    fn drag_divider(
        &mut self,
        drag: &DividerDrag,
        bounds: gpui::Bounds<gpui::Pixels>,
        position: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(Layout::Split {
            direction,
            children,
            ..
        }) = self.layout_tree.node_at(&drag.path)
        else {
            return;
        };
        let n = children.len();
        if drag.before + 1 >= n {
            return;
        }
        let row = *direction == SplitDirection::Vertical;
        let (origin, size) = if row {
            (bounds.origin.x, bounds.size.width)
        } else {
            (bounds.origin.y, bounds.size.height)
        };
        let along = if row { position.x } else { position.y };
        let size = f32::from(size);
        if size <= 0.0 {
            return;
        }
        let at = (f32::from(along - origin) / size).clamp(0.0, 1.0) as f64 * 100.0;
        let mut dims: Vec<f64> = match children
            .iter()
            .map(|c| self.dimension_of(c))
            .collect::<Option<Vec<f64>>>()
        {
            Some(dims) => dims,
            None => vec![100.0 / n as f64; n],
        };
        let before: f64 = dims[..drag.before].iter().sum();
        let pair = dims[drag.before] + dims[drag.before + 1];
        const MIN: f64 = 5.0;
        if pair <= 2.0 * MIN {
            return;
        }
        let a = (at - before).clamp(MIN, pair - MIN);
        dims[drag.before] = a;
        dims[drag.before + 1] = pair - a;
        for (ix, d) in dims.into_iter().enumerate() {
            let mut child = drag.path.clone();
            child.push(ix);
            self.set_dimension_at(&child, Some(d));
        }
        cx.notify();
    }

    /// `editor:focus-left` and its three siblings: the keyboard moves to the
    /// group on that side — of the groups there, the one used most recently,
    /// which is where Obsidian's went.
    fn focus_neighbor(&mut self, direction: SplitDirection, forward: bool, cx: &mut Context<Self>) {
        let candidates = self
            .layout_tree
            .neighbors(self.active_group_id(), direction, forward);
        let target = self
            .group_mru
            .iter()
            .find(|g| candidates.contains(g))
            .copied()
            .or_else(|| candidates.first().copied());
        if let Some(target) = target {
            self.activate_group(target, cx);
        }
    }

    /// Every tab but the live one: the active group's other tabs and all of
    /// the other groups'. What the live one needs is done on the workspace's
    /// own fields.
    fn stashed_tabs_mut(&mut self) -> impl Iterator<Item = &mut Tab> {
        let active_tab = self.active_tab;
        let active_group = self.active_group;
        let own = self
            .tabs
            .iter_mut()
            .enumerate()
            .filter(move |(ix, _)| *ix != active_tab)
            .map(|(_, t)| t);
        let others = self
            .groups
            .iter_mut()
            .enumerate()
            .filter(move |(gx, _)| *gx != active_group)
            .flat_map(|(_, g)| g.tabs.iter_mut());
        own.chain(others)
    }

    /// The group whose area the one graph view paints in: the active group
    /// while it shows the graph, else the first group that does.
    fn graph_group(&self) -> Option<u64> {
        if self.mode == ViewMode::Graph {
            return Some(self.active_group_id());
        }
        let mut order = Vec::new();
        self.layout_tree.groups(&mut order);
        order.into_iter().find(|id| {
            self.group_ix(*id).is_some_and(|ix| {
                let g = &self.groups[ix];
                g.tabs
                    .get(g.active_tab)
                    .is_some_and(|t| t.mode == ViewMode::Graph)
            })
        })
    }

    /// A sidebar's collapse toggle: it stays on screen when the sidebar is
    /// collapsed, which is what brings the sidebar back.
    fn sidebar_toggle(
        &self,
        left: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let visible = if left {
            self.left_visible
        } else {
            self.right_visible
        };
        let command = if left {
            "app:toggle-left-sidebar"
        } else {
            "app:toggle-right-sidebar"
        };
        let _ = visible;
        icon_button(
            if left { "toggle-left" } else { "toggle-right" },
            if left { "panel-left" } else { "panel-right" },
            theme,
        )
        .on_click(cx.listener(move |this, _, window, cx| this.run_command(command, window, cx)))
    }

    /// Whether `appearance.json` keeps the ribbon on screen.
    fn show_ribbon(&self) -> bool {
        self.state
            .as_ref()
            .map(|s| s.appearance().show_ribbon.unwrap_or(true))
            .unwrap_or(true)
    }

    /// Whether the window has macOS's hidden frame: the desktop's traffic
    /// lights in the title row's top-left corner, no title and no window
    /// buttons of Silica's. Obsidian keeps the sidebar toggles in the
    /// headers there — the left one in the left dock's header, or at the
    /// head of the root tab strip while that dock is collapsed, never above
    /// the ribbon — and the right one in the window's top-right corner,
    /// inside the right dock's own header while that dock is open.
    fn mac_frame(&self) -> bool {
        cfg!(target_os = "macos") && self.frameless
    }

    /// What the header in the window's top-left corner is padded by past
    /// its usual 8 px: Obsidian's `--frame-left-space`, room for the
    /// traffic lights on macOS and nothing elsewhere or in fullscreen,
    /// where the lights are gone.
    fn frame_left_space(&self) -> f32 {
        if !self.mac_frame() || self.fullscreen {
            return 0.;
        }
        MAC_FRAME_LEFT_SPACE - if self.show_ribbon() { RIBBON_WIDTH } else { 0. }
    }

    /// A sidebar's resize handle: the 3 px strip on its inner edge, which
    /// Obsidian colours with the accent while the pointer is over it.
    fn dock_handle(&self, left: bool, theme: &Theme) -> gpui::Stateful<gpui::Div> {
        div()
            .id(if left {
                "left-dock-handle"
            } else {
                "right-dock-handle"
            })
            .absolute()
            .top_0()
            .h_full()
            .w(px(3.))
            .when(left, |d| d.right(px(0.)).border_r_1())
            .when(!left, |d| d.left(px(0.)).border_l_1())
            .border_color(theme.border)
            .cursor_col_resize()
            .occlude()
            .hover(|s| s.bg(theme.accent).border_color(theme.accent))
            .on_drag(DockDrag { left }, |_, _, _, cx| cx.new(|_| NoDragPreview))
    }

    /// A sidebar handle dragged to `position` across the row of `bounds`:
    /// the dock follows the pointer, between a header row's worth and what
    /// leaves the main area room.
    fn drag_dock(
        &mut self,
        left: bool,
        bounds: gpui::Bounds<Pixels>,
        position: gpui::Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let x = f64::from(f32::from(position.x));
        let row = f64::from(f32::from(bounds.size.width));
        let max = (row - 2. * MIN_SIDEBAR_WIDTH).max(MIN_SIDEBAR_WIDTH);
        if left {
            let ribbon = if self.show_ribbon() {
                RIBBON_WIDTH as f64
            } else {
                0.
            };
            let origin = f64::from(f32::from(bounds.origin.x)) + ribbon;
            self.left_width = (x - origin).clamp(MIN_SIDEBAR_WIDTH, max);
        } else {
            let right = f64::from(f32::from(bounds.origin.x + bounds.size.width));
            self.right_width = (right - x).clamp(MIN_SIDEBAR_WIDTH, max);
        }
        cx.notify();
    }

    /// The window's own buttons in its top right corner, where Obsidian's
    /// hidden frame keeps them: minimise, maximise or restore, and close,
    /// each 44 by 40.
    /// The minimise, maximise and close buttons in the window's right
    /// corner, drawn only when the frame is Silica's.
    ///
    /// Windows wants these three named to the hit test rather than clicked:
    /// the system then performs them itself, and hovering the maximise
    /// button opens Windows 11's snap layouts, which an ordinary click
    /// handler can never offer. `occlude` is what keeps the press from
    /// reaching the caption area underneath and dragging the window instead.
    fn render_window_controls(&self, theme: &Theme, window: &Window) -> gpui::Div {
        let maximized = window.is_maximized();
        let system = cfg!(target_os = "windows");
        let corners = self.corners;
        let button = |id: &'static str,
                      name: &'static str,
                      close: bool,
                      area: WindowControlArea,
                      act: fn(&mut Window)| {
            let hover_bg: Hsla = if close {
                rgb(0xe93147).into()
            } else {
                theme.bg_alt
            };
            let color = theme.text_muted;
            div()
                .id(id)
                .group(id)
                .w(px(RIBBON_WIDTH))
                .h(px(HEADER_HEIGHT))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .when(close && corners.tr, |d| d.rounded_tr(WINDOW_RADIUS))
                .hover(move |s| s.bg(hover_bg))
                .when(system, |d| d.occlude().window_control_area(area))
                .when(!system, |d| d.on_click(move |_, window, _| act(window)))
                .child(
                    div()
                        .when(close, |d| {
                            d.group_hover(id, |s| s.text_color(gpui::white()))
                        })
                        .text_color(color)
                        .child(icon(name, 14., color)),
                )
        };
        div()
            .absolute()
            .top_0()
            .right_0()
            .flex()
            .flex_row()
            .child(button(
                "win-minimize",
                "minus",
                false,
                WindowControlArea::Min,
                |w| w.minimize_window(),
            ))
            .child(button(
                "win-maximize",
                if maximized { "copy" } else { "square" },
                false,
                WindowControlArea::Max,
                |w| w.zoom_window(),
            ))
            .child(button(
                "win-close",
                "x",
                true,
                WindowControlArea::Close,
                |w| w.remove_window(),
            ))
    }

    /// The ribbon down the left edge: the quick switcher, the graph, a new
    /// canvas, today's daily note, a template and the palette. `workspace.json`'s
    /// `left-ribbon.hiddenItems` hides an entry by the plugin id its key starts
    /// with. Settings is not here: Obsidian 1.14 leaves `.side-dock-settings`
    /// empty and puts the gear in the sidebar's footer, the vault profile.
    fn render_ribbon(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let hidden = |plugin: &str| -> bool {
            self.layout
                .as_ref()
                .and_then(|l| l.get("left-ribbon")?.get("hiddenItems")?.as_object())
                .is_some_and(|items| {
                    items.iter().any(|(key, hidden)| {
                        key.split(':').next() == Some(plugin) && hidden.as_bool() == Some(true)
                    })
                })
        };
        let enabled = |plugin: &str| -> bool {
            self.state
                .as_ref()
                .is_some_and(|s| settings::plugin_enabled(s, plugin))
        };
        // The corner above the ribbon is part of the title row: it carries
        // the headers' bottom line across, and the ribbon's own divider
        // starts below it, since Obsidian's ribbon begins under the title
        // row. The left sidebar's toggle sits in the corner, which is where
        // Obsidian's hidden frame puts it on Linux and Windows. On macOS the
        // corner is the traffic lights' and only drags the window; the
        // toggle is in the headers instead.
        let toggle: gpui::AnyElement = if self.mac_frame() {
            window_handle(
                div()
                    .id("ribbon-corner")
                    .w(px(RIBBON_WIDTH))
                    .h(px(HEADER_HEIGHT))
                    .border_b_1()
                    .border_color(theme.border),
                true,
            )
            .into_any_element()
        } else {
            div()
                .w(px(RIBBON_WIDTH))
                .h(px(HEADER_HEIGHT))
                .flex()
                .items_center()
                .justify_center()
                .border_b_1()
                .border_color(theme.border)
                .child(self.sidebar_toggle(true, theme, cx))
                .into_any_element()
        };
        let button = |id: &'static str, name: &'static str, command: &'static str| {
            icon_button(id, name, theme).on_click(
                cx.listener(move |this, _, window, cx| this.run_command(command, window, cx)),
            )
        };
        // The icons are the ones Obsidian's ribbon names: `lucide-file-search`
        // for the switcher, `git-fork` for the graph, `layout-dashboard` for
        // a new canvas, `calendar`, `files` for a template, `terminal` for
        // the palette and `layout-list` for a new base -- in that order,
        // which is the order the DOM was read in.
        let items: [(&'static str, &'static str, &'static str, &'static str); 7] = [
            (
                "ribbon-switcher",
                "file-search",
                "switcher:open",
                "switcher",
            ),
            ("ribbon-graph", "git-fork", "graph:open", "graph"),
            (
                "ribbon-canvas",
                "layout-dashboard",
                "canvas:new-file",
                "canvas",
            ),
            ("ribbon-daily", "calendar", "daily-notes", "daily-notes"),
            ("ribbon-template", "files", "insert-template", "templates"),
            (
                "ribbon-palette",
                "terminal",
                "command-palette:open",
                "command-palette",
            ),
            ("ribbon-base", "layout-list", "bases:new-file", "bases"),
        ];
        let buttons: Vec<_> = items
            .into_iter()
            .filter(|(_, _, _, plugin)| enabled(plugin) && !hidden(plugin))
            .map(|(id, name, command, _)| button(id, name, command))
            .collect();
        div()
            .flex()
            .flex_col()
            .items_center()
            .flex_shrink_0()
            .w(px(RIBBON_WIDTH))
            .h_full()
            .bg(theme.bg_secondary)
            .map(|d| self.corners.left().apply(d))
            .child(toggle)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .flex_1()
                    .w_full()
                    .border_r_1()
                    .border_color(theme.border)
                    .pt(px(8.))
                    .pb(px(12.))
                    .gap(px(6.))
                    .children(buttons),
            )
    }

    /// The left sidebar's footer, Obsidian's vault profile: the vault's
    /// name behind a `chevrons-up-down` switcher on the left, Help and
    /// Settings on the right, above a hairline. It is part of the sidebar,
    /// so it collapses with it, and it stays whether the ribbon shows or not.
    fn render_vault_profile(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let menu = self.render_vault_menu(theme, window, cx);
        let button = |id: &'static str, name: &'static str, command: &'static str| {
            icon_button(id, name, theme).on_click(
                cx.listener(move |this, _, window, cx| this.run_command(command, window, cx)),
            )
        };
        div()
            .flex()
            .flex_row()
            .items_center()
            .flex_shrink_0()
            .h(px(HEADER_HEIGHT))
            .px(px(8.))
            .gap(px(2.))
            .border_t_1()
            .border_color(theme.border)
            .child(
                div()
                    .id("vault-switcher")
                    .flex()
                    .flex_row()
                    .items_center()
                    .min_w_0()
                    .flex_1()
                    .h(px(24.))
                    .pr(px(6.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.bg_alt))
                    .when(self.vault_menu_open, |d| d.bg(theme.bg_alt))
                    .on_hover(cx.listener(|this, over: &bool, _, _| {
                        this.vault_switcher_hovered = *over;
                    }))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_vault_menu(cx)))
                    // The menu's anchor is this box, pinned at the row's
                    // top-left corner: an absolute element left to the
                    // row's own centring would hang from its middle.
                    .child(div().absolute().top_0().left_0().size_0().children(menu))
                    .child(
                        div()
                            .flex_shrink_0()
                            .w(px(28.))
                            .h(px(24.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon("chevrons-up-down", ICON_SIZE, theme.text_muted)),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_size(px(FONT_UI_SMALL))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(theme.text)
                            .child(self.vault_name.clone()),
                    ),
            )
            .child(button("vault-help", "circle-help", "app:open-help"))
            .child(button("vault-settings", "settings", "app:open-settings"))
    }

    /// The vault switcher's menu, dropped up from the footer: every vault
    /// this machine has opened whose folder is still there, in the
    /// registry's own order with a check on the one showing, then
    /// "Manage vaults...". A folder that is gone is left out rather than
    /// listed faint, and the row is the folder's name alone however many
    /// vaults share it — both are what Obsidian's own switcher does
    /// (probe of 2026-10-29); the path is the row's tooltip. A click
    /// outside closes it, and so does a second click on the footer's own
    /// row.
    fn render_vault_menu(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if !self.vault_menu_open {
            return None;
        }
        let current = self.state.as_ref().map(|s| s.root.clone());
        let registry = VaultRegistry::open().ok().flatten();
        let known: Vec<(String, PathBuf)> = registry
            .as_ref()
            .map(|r| {
                r.vaults()
                    .into_iter()
                    .filter(|(_, e)| e.path.is_dir())
                    .map(|(_, e)| (e.name(), e.path.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let item = |id: gpui::ElementId, theme: &Theme| {
            div()
                .id(id)
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .px_3()
                .py(px(5.))
                .rounded(px(4.))
                .mx_1()
                .cursor_pointer()
                .hover(|s| s.bg(theme.bg_alt))
        };
        let check = |on: bool, theme: &Theme| {
            div()
                .w(px(16.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .children(on.then(|| icon("check", 14., theme.text_muted)))
        };
        // A machine that has opened dozens of vaults has a menu taller than
        // the window, and the rows under the hairline are then off the
        // bottom edge however the menu is snapped: cap it at the window and
        // scroll, as Obsidian's own menus do inside their `.menu-scroll`.
        let tallest = (window.viewport_size().height - px(96.)).max(px(160.));
        let mut list = div()
            .id("vault-menu")
            .flex()
            .flex_col()
            .min_w(px(240.))
            .max_w(px(360.))
            .max_h(tallest)
            .overflow_y_scroll()
            .py_1()
            .bg(theme.bg_secondary)
            .border_1()
            .border_color(theme.border)
            .rounded_md()
            .shadow_lg()
            .text_size(px(FONT_UI_SMALL))
            .text_color(theme.text)
            .occlude()
            // A press on the switcher row is a press outside the menu, and
            // the click that ends it toggles: closing here as well would
            // close and reopen the menu in one press. The row's own
            // handler does the closing when the press is its.
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                if this.vault_switcher_hovered {
                    return;
                }
                this.vault_menu_open = false;
                cx.notify();
            }));
        if known.is_empty() {
            list = list.child(
                div()
                    .px_3()
                    .py(px(5.))
                    .text_color(theme.text_faint)
                    .child("No vaults opened yet"),
            );
        }
        for (ix, (name, path)) in known.into_iter().enumerate() {
            let showing = current.as_deref() == Some(path.as_path());
            let tooltip = crate::canvas::menu::tooltip(path.display().to_string());
            list = list.child(
                item(("vault-menu-item", ix).into(), theme)
                    .tooltip(tooltip)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.vault_menu_open = false;
                        if showing {
                            cx.notify();
                        } else if path.is_dir() {
                            this.load_vault(path.clone(), window, cx);
                        } else {
                            this.report(format!("{} is not a folder any more", path.display()), cx);
                        }
                    }))
                    .child(check(showing, theme))
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(name),
                    ),
            );
        }
        // Obsidian's tail is a single row under the separator, "Manage
        // vaults...", which raises the vault manager (probe of
        // 2026-10-29). Silica's own "Open vault..." and "Open sandbox
        // vault" moved into that window, where Obsidian keeps the same
        // two; they are still commands, and still in the palette.
        list =
            list.child(div().my_1().h(px(1.)).bg(theme.border)).child(
                item("vault-menu-manage".into(), theme)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.run_command("app:open-vault", window, cx)
                    }))
                    .child(
                        div()
                            .w(px(16.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .child(icon("folder-open", 14., theme.text_muted)),
                    )
                    .child("Manage vaults..."),
            );
        Some(
            deferred(
                anchored()
                    .anchor(gpui::Anchor::BottomLeft)
                    .offset(point(px(0.), px(-6.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(list),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    /// A group's tab strip. The sidebar toggles sit at the window's edges:
    /// the left one in the first group's strip, the right one in the last's.
    /// `first` and `last` say the group holds the main area's top left and
    /// top right corner: those strips carry the collapsed sidebars' toggles
    /// and clear the window controls.
    fn render_tab_strip(
        &self,
        view: &GroupView,
        first: bool,
        last: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let tabs = &view.tabs;
        let active_ix = view.active_tab;
        let group = view.id;
        let group_active = view.active;
        let show_ribbon = self.show_ribbon();
        let frameless = self.frameless;
        let mac = self.mac_frame();
        // The window buttons overlay the right end of whichever header is
        // in the window's top right corner: this strip's, when the right
        // sidebar is collapsed and this is the last group. macOS has none.
        let under_controls = frameless && !mac && last && !self.right_visible;
        // The strip in the window's top-left corner, with the left sidebar
        // collapsed, keeps clear of the traffic lights on macOS.
        let left_space = if first && !self.left_visible {
            self.frame_left_space()
        } else {
            0.
        };
        let menu_open = self.tab_list_menu == Some(group);
        let at_left = first && !self.show_ribbon() && !self.left_visible;
        let at_right = last && !self.right_visible;
        div()
            .flex()
            .flex_row()
            .items_end()
            .when(at_left && self.corners.tl, |d| d.rounded_tl(WINDOW_RADIUS))
            .when(at_right && self.corners.tr, |d| d.rounded_tr(WINDOW_RADIUS))
            .h(px(HEADER_HEIGHT))
            .pt(px(6.))
            .pl(px(8. + left_space))
            .pr(if under_controls {
                px(3. * RIBBON_WIDTH)
            } else {
                px(8.)
            })
            .bg(theme.bg_secondary)
            .relative()
            .overflow_hidden()
            // A dragged tab over the strip moves the drop bar to the
            // nearest tab boundary; dropped, it lands there (at the end,
            // should the bar never have found one).
            .on_drag_move(cx.listener(move |this, e: &DragMoveEvent<TabDrag>, _, cx| {
                this.strip_drag_moved(group, e.bounds, e.event.position, cx)
            }))
            .on_drop(cx.listener(move |this, drag: &TabDrag, _, cx| {
                this.drop_tab_at_bar(drag, group, None, cx)
            }))
            // The strip's outline is a hairline along its bottom edge rather
            // than a border, so the active tab, which reaches the edge, paints
            // over it and joins the view header below without a seam. The
            // other tabs stop a pixel short and keep the line under them.
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(px(1.))
                    .bg(theme.border),
            )
            // Without the ribbon a collapsed left sidebar would have no
            // way back from the window; macOS keeps the toggle here, after
            // the traffic lights, ribbon or not.
            .when(first && !self.left_visible && (!show_ribbon || mac), |d| {
                d.child(self.sidebar_toggle(true, theme, cx).mb(px(8.)).mr(px(2.)))
            })
            .children(tabs.iter().enumerate().map(|(ix, tab)| {
                let active = ix == active_ix;
                let divider = !active && ix + 1 != active_ix;
                let title = tab.title();
                let drag = TabDrag {
                    group,
                    tab: ix,
                    title: title.clone(),
                };
                let this = cx.weak_entity();
                // Obsidian's tab: 34 px tall, 200 px wide, the active one
                // in the content's colour, its top corners as round as its
                // feet, its title in the small UI size and its close button a
                // 20 px disc. It is padded `1px 4px 4px` around an inner box
                // with a 4 px radius, and that inner box, not the tab, is
                // what the hover wash paints.
                div()
                    .id(("tab", ix))
                    .group("tab")
                    .flex()
                    .flex_row()
                    .h(px(34.))
                    .w(px(200.))
                    .min_w(px(60.))
                    .flex_shrink()
                    .pt(px(1.))
                    .px(px(4.))
                    .pb(px(4.))
                    .rounded_t(px(TAB_CURVE))
                    .relative()
                    .when(!active, |d| d.mb(px(1.)))
                    // The divider on an inactive tab's right edge, as
                    // Obsidian's `::after`: a 1 px line, 15 px tall and 8 px
                    // down, astride the edge (the tabs touch, so it is
                    // centred between two inner boxes, 3.5 px from each),
                    // and after the last tab it parts the strip from the
                    // new-tab button. The active tab has none and paints
                    // over its left neighbour's, so the gaps around it are
                    // bare. Hover leaves it alone: the wash stops 4 px short
                    // of the edge, so the line stays beside the hovered box.
                    // Deferred, like the feet: the next tab paints after
                    // this one with an opaque background, and the half of
                    // the line astride its edge would otherwise be lost
                    // under it, whole or in part depending on where the
                    // edge falls between device pixels.
                    .when(divider, |d| {
                        d.child(deferred(
                            div()
                                .absolute()
                                .right(px(-0.5))
                                .top(px(8.))
                                .w(px(1.))
                                .h(px(15.))
                                .bg(theme.tab_divider),
                        ))
                    })
                    // The active tab's feet: the concave curves that carry
                    // its bottom corners out into the view header. Painted
                    // late so a neighbour's hover wash stays underneath.
                    .when(active, |d| {
                        d.child(deferred(tab_foot(true, theme)))
                            .child(deferred(tab_foot(false, theme)))
                    })
                    .cursor_pointer()
                    .text_size(px(FONT_UI_SMALL))
                    .bg(if active { theme.bg } else { theme.bg_secondary })
                    .text_color(if active && group_active {
                        theme.text
                    } else if active {
                        theme.text_muted
                    } else {
                        theme.text_faint
                    })
                    .when(!active, |d| d.hover(|s| s.text_color(theme.text_muted)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.activate_group(group, cx);
                        this.activate_tab(ix, cx)
                    }))
                    // Picking the tab up: the ghost is its title, and the
                    // workspace is told so every group shows its zones.
                    .on_drag(drag, move |drag, grab, _, cx| {
                        this.update(cx, |w, cx| w.begin_tab_drag(cx)).ok();
                        let title = drag.title.clone();
                        cx.new(|_| TabDragPreview { title, grab })
                    })
                    // Where the tab is, for the strip's drop bar; a tab
                    // dropped on it lands at the bar, else in front of it.
                    .on_drag_move(cx.listener(move |this, e: &DragMoveEvent<TabDrag>, _, _| {
                        if this.dragging_tab {
                            this.tab_bounds.insert((group, ix), e.bounds);
                        }
                    }))
                    .on_drop(cx.listener(move |this, drag: &TabDrag, _, cx| {
                        this.drop_tab_at_bar(drag, group, Some(ix), cx)
                    }))
                    .on_mouse_down(
                        gpui::MouseButton::Middle,
                        cx.listener(move |this, _, _, cx| {
                            this.activate_group(group, cx);
                            this.close_tab(ix, cx)
                        }),
                    )
                    // The inner box: the title and the close button, padded
                    // `0 3px 0 6px`, washed on hover when the tab is not the
                    // active one (the active tab shows nothing on hover).
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(2.))
                            .pl(px(6.))
                            .pr(px(3.))
                            .rounded(px(4.))
                            .when(!active, |d| d.group_hover("tab", |s| s.bg(theme.bg_alt)))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(title),
                            )
                            // The close button: no wash of its own, its cross
                            // brightens from muted to normal under the pointer.
                            .child(
                                div()
                                    .id(("tab-close", ix))
                                    .group("tab-close")
                                    .flex_shrink_0()
                                    .w(px(20.))
                                    .h(px(20.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(8.))
                                    .when(!active, |d| {
                                        d.invisible().group_hover("tab", |s| s.visible())
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.activate_group(group, cx);
                                        this.close_tab(ix, cx);
                                    }))
                                    .child(
                                        icon("x", 14., theme.text_muted)
                                            .group_hover("tab-close", |s| s.text_color(theme.text)),
                                    ),
                            ),
                    )
            }))
            .child(
                div()
                    .id("new-tab")
                    .flex_shrink_0()
                    .ml(px(8.))
                    .mr(px(4.))
                    .mb(px(7.))
                    .w(px(24.))
                    .h(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.bg_alt))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.activate_group(group, cx);
                        this.new_tab(cx)
                    }))
                    .child(icon("plus", ICON_SIZE, theme.text)),
            )
            .child(window_handle(
                div().id(("tab-strip-space", group)).flex_1().h_full(),
                frameless,
            ))
            // The tab-list button at the strip's right end: Obsidian's
            // `chevron-down`, 24 px square at y 8.66 with a 2 px right
            // margin. The menu hangs off the wrapper, so it opens under
            // the button's top-left corner, 2 px below the button.
            .child(
                div()
                    .relative()
                    .flex_shrink_0()
                    .mr(px(2.))
                    .mb(px(7.))
                    .child(
                        div()
                            .id(("tab-list", group))
                            .w(px(24.))
                            .h(px(24.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme.bg_alt))
                            .when(menu_open, |d| d.bg(theme.bg_alt))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_tab_list_menu(group, cx)
                            }))
                            .child(icon("chevron-down", ICON_SIZE, theme.text)),
                    )
                    .children(self.render_tab_list_menu(view, theme, cx)),
            )
            // The right sidebar's toggle sits at the root header's right
            // end in Obsidian's title row whether the sidebar is open (left
            // of its header) or collapsed (at the window's edge, before the
            // window buttons): it never moves into the sidebar's own header.
            // On macOS it keeps to the window's top-right corner, which is
            // the right dock's own header while that dock is open.
            .when(last && !(mac && self.right_visible), |d| {
                d.child(self.sidebar_toggle(false, theme, cx).mb(px(8.)))
            })
            // The drop bar: an accent bar astride the boundary the dragged
            // tab would land at, the height of the tabs' inner boxes.
            // Deferred past the dividers and feet, which paint late too.
            .when_some(self.drop_bar.filter(|b| b.group == group), |d, bar| {
                let left = bar.shown_x(Instant::now()) - bar.strip_x - DROP_BAR_WIDTH / 2.;
                d.child(deferred(
                    div()
                        .absolute()
                        .left(px(left))
                        .bottom(px(6.))
                        .w(px(DROP_BAR_WIDTH))
                        .h(px(25.))
                        .rounded(px(DROP_BAR_WIDTH / 2.))
                        .bg(theme.accent),
                ))
            })
    }

    fn toggle_tab_list_menu(&mut self, group: u64, cx: &mut Context<Self>) {
        self.tab_list_menu = if self.tab_list_menu == Some(group) {
            None
        } else {
            Some(group)
        };
        cx.notify();
    }

    /// The tab-list menu, as Obsidian 1.14 drops it from the strip's
    /// `chevron-down` (probed 2026-10-22): "Stack tabs" (`layers`), "Bookmark
    /// N tabs..." (`bookmark`, only when a tab has a file), a separator,
    /// "Close all" (`x`), a separator, then one row per tab (`file`) with a
    /// check on the active one. Rows are 25 px, padded `4px 8px` with an
    /// 8 px gap, in a menu padded 6 px whose separators run its full width.
    fn render_tab_list_menu(
        &self,
        view: &GroupView,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if self.tab_list_menu != Some(view.id) {
            return None;
        }
        let group = view.id;
        let active_ix = view.active_tab;
        let paths: Vec<String> = view.tabs.iter().filter_map(|t| t.path.clone()).collect();
        let row = |id: gpui::ElementId, name: &'static str, theme: &Theme| {
            div()
                .id(id)
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .h(px(25.))
                .rounded(px(4.))
                .cursor_pointer()
                .hover(|s| s.bg(theme.bg_alt))
                .child(icon(name, ICON_SIZE, theme.text_muted))
        };
        let separator = |theme: &Theme| div().my(px(6.)).mx(px(-6.)).h(px(1.)).bg(theme.border);
        let mut list = div()
            .id(("tab-list-menu", group))
            .flex()
            .flex_col()
            .min_w(px(167.))
            .max_w(px(360.))
            .p(px(6.))
            .bg(theme.bg_secondary)
            .border_1()
            .border_color(theme.border)
            .rounded(px(8.))
            .shadow_lg()
            .text_size(px(FONT_UI_SMALL))
            .text_color(theme.text)
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.tab_list_menu = None;
                cx.notify();
            }))
            .child(
                row("tab-list-stack".into(), "layers", theme)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.tab_list_menu = None;
                        this.run_command("workspace:toggle-stacked-tabs", window, cx);
                    }))
                    .child("Stack tabs"),
            );
        if !paths.is_empty() {
            let label = if paths.len() == 1 {
                "Bookmark 1 tab...".to_string()
            } else {
                format!("Bookmark {} tabs...", paths.len())
            };
            list = list.child(
                row("tab-list-bookmark".into(), "bookmark", theme)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.tab_list_menu = None;
                        this.start_bookmark(BookmarkDraft::Tabs(paths.clone()), window, cx);
                    }))
                    .child(label),
            );
        }
        list = list
            .child(separator(theme))
            .child(
                row("tab-list-close-all".into(), "x", theme)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tab_list_menu = None;
                        this.close_group(group, true, cx);
                    }))
                    .child("Close all"),
            )
            .child(separator(theme));
        for (ix, tab) in view.tabs.iter().enumerate() {
            let checked = ix == active_ix;
            list = list.child(
                row(("tab-list-tab", ix).into(), "file", theme)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tab_list_menu = None;
                        this.activate_group(group, cx);
                        this.activate_tab(ix, cx);
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(tab.title()),
                    )
                    .children(checked.then(|| icon("check", ICON_SIZE, theme.text_muted))),
            );
        }
        // The wrapper lays the menu out below the button, so the offset
        // is the gap under it.
        Some(
            deferred(
                anchored()
                    .offset(point(px(0.), px(2.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(list),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    // ----- workspace.json --------------------------------------------------------

    /// Whether a `workspace.json` was read at load. The startup flags then
    /// leave the first note alone: with nothing recorded Obsidian shows the
    /// empty state rather than picking a note.
    pub fn layout_restored(&self) -> bool {
        self.layout_restored
    }

    /// Restores the layout `workspace.json` records: the main area's tab
    /// groups and how they are split, the active leaf, each sidebar's
    /// collapse state and its tab.
    fn restore_layout(
        &mut self,
        state: &Arc<VaultState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let layout = match state.vault().workspace_json() {
            Ok(Some(layout)) => layout,
            Ok(None) => return,
            Err(err) => {
                self.report(format!("workspace.json could not be read: {err}"), cx);
                return;
            }
        };
        self.layout_restored = true;
        if let Some((sort, auto_reveal)) = explorer_leaf_state(&layout) {
            self.explorer
                .update(cx, |e, cx| e.set_leaf_state(sort, auto_reveal, cx));
        }
        self.restore_pane_states(&layout, cx);
        self.last_open_files = layout
            .get("lastOpenFiles")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let default_mode = self.default_mode();
        let spec = layout.get("main").and_then(|main| {
            // A leaf naming a file the vault no longer has is dropped, the
            // way Obsidian drops one it cannot open.
            read_main(main, default_mode, &|file| {
                state.vault().absolute(file).is_file()
            })
        });
        if let Some(spec) = spec {
            // The groups the file describes replace the one the window
            // opened with; its views were empty.
            let placeholder = self.active_group_id();
            let mut leaf_ids: Vec<(String, u64, usize)> = Vec::new();
            let tree = self.materialize(spec, &mut leaf_ids, window, cx);
            self.layout_tree = tree;
            if let Some(ix) = self.group_ix(placeholder) {
                self.groups.remove(ix);
                self.group_mru.retain(|g| *g != placeholder);
            }
            let active_id = layout.get("active").and_then(Value::as_str);
            let (group, tab) = leaf_ids
                .iter()
                .find(|(id, _, _)| Some(id.as_str()) == active_id)
                .map(|(_, g, t)| (*g, Some(*t)))
                .or_else(|| {
                    // No active leaf on record: the first with a file.
                    leaf_ids
                        .iter()
                        .find(|(_, g, t)| {
                            self.group_ix(*g)
                                .is_some_and(|ix| self.groups[ix].tabs[*t].path.is_some())
                        })
                        .map(|(_, g, t)| (*g, Some(*t)))
                })
                .or_else(|| leaf_ids.first().map(|(_, g, _)| (*g, None)))
                .unwrap_or((self.groups[0].id, None));
            if let (Some(ix), Some(tab)) = (self.group_ix(group), tab) {
                self.groups[ix].active_tab = tab;
            }
            // Fill every group's views, the active one last so its note is
            // the one the explorer, graph and right dock follow.
            let mut order = Vec::new();
            self.layout_tree.groups(&mut order);
            for id in order.into_iter().filter(|id| *id != group).chain([group]) {
                if let Some(ix) = self.group_ix(id) {
                    self.active_group = ix;
                    self.load_group(ix, cx);
                    self.load_tab(cx);
                    self.stash_active_group();
                }
            }
            // The most recently used order Obsidian would have: the active
            // group first, the rest in layout order.
            self.group_mru = {
                let mut mru = vec![group];
                let mut order = Vec::new();
                self.layout_tree.groups(&mut order);
                mru.extend(order.into_iter().filter(|g| *g != group));
                mru
            };
        }
        let collapsed = |dock: &str| {
            layout
                .get(dock)
                .and_then(|d| d.get("collapsed"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        };
        self.left_visible = !collapsed("left");
        self.right_visible = !collapsed("right");
        // A dock's `width` is where its handle was last dragged to; a
        // fresh vault has 300, and nothing narrower than a header row's
        // worth of buttons is believed.
        let width = |dock: &str| {
            layout
                .get(dock)
                .and_then(|d| d.get("width"))
                .and_then(Value::as_f64)
                .filter(|w| *w >= MIN_SIDEBAR_WIDTH)
        };
        if let Some(w) = width("left") {
            self.left_width = w;
        }
        if let Some(w) = width("right") {
            self.right_width = w;
        }
        match dock_current_view(&layout, "left").as_deref() {
            Some("search") => self.left_tab = LeftTab::Search,
            Some("bookmarks") => self.left_tab = LeftTab::Bookmarks,
            Some(_) => self.left_tab = LeftTab::Files,
            None => {}
        }
        match dock_current_view(&layout, "right").as_deref() {
            Some("backlink") => self.right_tab = RightTab::Backlinks,
            Some("outgoing-link") => self.right_tab = RightTab::Outgoing,
            Some("outline") => self.right_tab = RightTab::Outline,
            Some("tag") => self.right_tab = RightTab::Tags,
            Some("all-properties") => self.right_tab = RightTab::Properties,
            _ => {}
        }
        self.layout = Some(layout);
    }

    /// Makes the groups a `main` node describes: each with its own views,
    /// its tabs in place, not yet shown. `leaf_ids` collects every restored
    /// leaf's id with the group and tab it became, for `active`.
    fn materialize(
        &mut self,
        spec: LayoutSpec,
        leaf_ids: &mut Vec<(String, u64, usize)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Layout {
        match spec {
            LayoutSpec::Group(group) => {
                let mode = group
                    .tabs
                    .first()
                    .map(|(_, t)| t.mode)
                    .unwrap_or(ViewMode::Reading);
                let id = self.new_group(mode, window, cx);
                let ix = self.group_ix(id).expect("the group was just made");
                for (tab_ix, (leaf, _)) in group.tabs.iter().enumerate() {
                    leaf_ids.push((leaf.clone(), id, tab_ix));
                }
                let g = &mut self.groups[ix];
                g.tabs = group.tabs.into_iter().map(|(_, t)| t).collect();
                g.active_tab = group.active_tab.min(g.tabs.len().saturating_sub(1));
                g.node = group.node;
                Layout::Group(id)
            }
            LayoutSpec::Split {
                direction,
                children,
                node,
            } => Layout::Split {
                direction,
                children: children
                    .into_iter()
                    .map(|c| self.materialize(c, leaf_ids, window, cx))
                    .collect(),
                node,
            },
        }
    }

    /// The Backlinks and Outgoing links leaves' own state from
    /// `workspace.json`'s right dock, each key at Obsidian's default when
    /// it is missing or unreadable.
    fn restore_pane_states(&mut self, layout: &Value, cx: &mut Context<Self>) {
        let flag =
            |state: &Value, key: &str| state.get(key).and_then(Value::as_bool).unwrap_or(false);
        if let Some(state) = leaf_state(layout, "left", "search") {
            self.search.update(cx, |s, cx| s.restore(state, cx));
        }
        if let Some(state) = leaf_state(layout, "right", "backlink") {
            let b = &mut self.backlink_state;
            b.collapse_all = flag(state, "collapseAll");
            b.extra_context = flag(state, "extraContext");
            b.sort = state
                .get("sortOrder")
                .and_then(Value::as_str)
                .and_then(SortOrder::parse)
                .unwrap_or_default();
            b.show_search = flag(state, "showSearch");
            b.linked_collapsed = flag(state, "backlinkCollapsed");
            b.unlinked_collapsed = flag(state, "unlinkedCollapsed");
            let query = state
                .get("searchQuery")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            self.backlink_filter
                .update(cx, |input, cx| input.set_text(query, cx));
        }
        if let Some(state) = leaf_state(layout, "right", "outgoing-link") {
            self.outgoing_state.links_collapsed = flag(state, "linksCollapsed");
            self.outgoing_state.unlinked_collapsed = flag(state, "unlinkedCollapsed");
        }
        let sort = |state: &Value| {
            state
                .get("sortOrder")
                .and_then(Value::as_str)
                .and_then(PaneSort::parse)
                .unwrap_or_default()
        };
        let query = |state: &Value| {
            state
                .get("searchQuery")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        if let Some(state) = leaf_state(layout, "right", "tag") {
            self.tag_state.sort = sort(state);
            self.tag_state.use_hierarchy = state
                .get("useHierarchy")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            self.tag_state.show_search = flag(state, "showSearch");
            let q = query(state);
            self.tag_filter
                .update(cx, |input, cx| input.set_text(q, cx));
        }
        if let Some(state) = leaf_state(layout, "right", "all-properties") {
            self.property_state.sort = sort(state);
            self.property_state.show_search = flag(state, "showSearch");
            let q = query(state);
            self.property_filter
                .update(cx, |input, cx| input.set_text(q, cx));
        }
        if let Some(state) = leaf_state(layout, "right", "outline") {
            self.outline_state.follow_cursor = flag(state, "followCursor");
            self.outline_state.show_search = flag(state, "showSearch");
            let q = query(state);
            self.outline_filter
                .update(cx, |input, cx| input.set_text(q, cx));
        }
    }

    /// `workspace.json` as this session would leave it: the file that was
    /// read with the tab groups, the active leaf, the sidebars' collapse
    /// state and `lastOpenFiles` brought up to date, and every other key as
    /// it was.
    fn layout_json(&self, cx: &App) -> Value {
        let mut layout = self.layout.clone().unwrap_or_else(|| json!({}));
        let Some(object) = layout.as_object_mut() else {
            return layout;
        };
        // A vault Obsidian never laid out has no docks in the file, or no
        // file at all. Obsidian writes its default ones; so does Silica, so
        // the panes' own state has a leaf to live in rather than being
        // lost, with `main` first the way Obsidian orders the keys.
        if !object.contains_key("main") {
            object.insert("main".into(), Value::Null);
        }
        let (left_tab, right_tab) = (self.left_tab as usize, self.right_tab as usize);
        for (dock, tab) in [("left", left_tab), ("right", right_tab)] {
            if !object.get(dock).is_some_and(Value::is_object) {
                object.insert(
                    dock.into(),
                    default_dock_json(dock, self.current.as_deref(), tab),
                );
            }
        }
        // The Files leaf's own state, where the file has that leaf.
        {
            let explorer = self.explorer.read(cx);
            let (sort, auto_reveal) = (explorer.sort_order(), explorer.auto_reveal());
            if let Some(left) = object.get_mut("left") {
                set_explorer_leaf_state(left, sort, auto_reveal);
                set_leaf_state(left, "search", &self.search.read(cx).leaf_state(cx));
            }
        }
        // The Backlinks and Outgoing links leaves' own state, likewise.
        if let Some(right) = object.get_mut("right") {
            let b = &self.backlink_state;
            set_leaf_state(
                right,
                "backlink",
                &[
                    ("collapseAll", json!(b.collapse_all)),
                    ("extraContext", json!(b.extra_context)),
                    ("sortOrder", json!(b.sort.as_str())),
                    ("showSearch", json!(b.show_search)),
                    ("searchQuery", json!(self.backlink_filter.read(cx).text())),
                    ("backlinkCollapsed", json!(b.linked_collapsed)),
                    ("unlinkedCollapsed", json!(b.unlinked_collapsed)),
                ],
            );
            let o = &self.outgoing_state;
            set_leaf_state(
                right,
                "outgoing-link",
                &[
                    ("linksCollapsed", json!(o.links_collapsed)),
                    ("unlinkedCollapsed", json!(o.unlinked_collapsed)),
                ],
            );
            // The Tags, All properties and Outline leaves' own state.
            let t = &self.tag_state;
            set_leaf_state(
                right,
                "tag",
                &[
                    ("sortOrder", json!(t.sort.as_str())),
                    ("useHierarchy", json!(t.use_hierarchy)),
                    ("showSearch", json!(t.show_search)),
                    ("searchQuery", json!(self.tag_filter.read(cx).text())),
                ],
            );
            let p = &self.property_state;
            set_leaf_state(
                right,
                "all-properties",
                &[
                    ("sortOrder", json!(p.sort.as_str())),
                    ("showSearch", json!(p.show_search)),
                    ("searchQuery", json!(self.property_filter.read(cx).text())),
                ],
            );
            // `file` is the open note; Obsidian leaves the key out for an
            // empty tab, and a `null` here removes it.
            let o = &self.outline_state;
            set_leaf_state(
                right,
                "outline",
                &[
                    ("file", json!(self.current)),
                    ("followCursor", json!(o.follow_cursor)),
                    ("showSearch", json!(o.show_search)),
                    ("searchQuery", json!(self.outline_filter.read(cx).text())),
                ],
            );
        }
        let mut active_id = Value::Null;
        let main = self.layout_node_json(&self.layout_tree, &mut active_id);
        // `main` keeps its place in a file that had one.
        match object.get_mut("main") {
            Some(slot) => *slot = main,
            None => {
                object.insert("main".into(), main);
            }
        }
        object.insert("active".into(), active_id);
        for (dock, visible, width) in [
            ("left", self.left_visible, self.left_width),
            ("right", self.right_visible, self.right_width),
        ] {
            if let Some(d) = object.get_mut(dock).and_then(Value::as_object_mut) {
                if visible {
                    d.shift_remove("collapsed");
                } else {
                    d.insert("collapsed".into(), json!(true));
                }
                // The width is rewritten only once a drag moved it, so the
                // `300` Obsidian wrote stays the integer it was and a file
                // without the key gains it only for a sidebar that moved.
                let same = d
                    .get("width")
                    .and_then(Value::as_f64)
                    .is_some_and(|w| (w - width).abs() < 0.5);
                if !same && (d.contains_key("width") || width != SIDEBAR_WIDTH as f64) {
                    d.insert("width".into(), json!(width));
                }
            }
        }
        object.insert("lastOpenFiles".into(), json!(self.last_open_files));
        layout
    }

    /// The layout tree for the socket's state dump: group ids at the leaves.
    fn layout_tree_json(&self, node: &Layout) -> Value {
        match node {
            Layout::Group(id) => json!({"group": id, "dimension": self.dimension_of(node)}),
            Layout::Split {
                direction,
                children,
                ..
            } => json!({
                "direction": direction.as_str(),
                "dimension": self.dimension_of(node),
                "children": children.iter().map(|c| self.layout_tree_json(c)).collect::<Vec<Value>>(),
            }),
        }
    }

    /// One node of `main`: a group as a `tabs` node with its leaves and
    /// `currentTab`, a split with its children and `direction`, each over
    /// the keys it was read with. The active group's active leaf id lands in
    /// `active_id` on the way, since a new tab's id exists only in the leaf
    /// written here.
    fn layout_node_json(&self, node: &Layout, active_id: &mut Value) -> Value {
        match node {
            Layout::Group(id) => {
                let Some(ix) = self.group_ix(*id) else {
                    return Value::Null;
                };
                let active = ix == self.active_group;
                let (tabs, active_tab) = if active {
                    (self.tab_snapshots(), self.active_tab)
                } else {
                    (self.groups[ix].tabs.clone(), self.groups[ix].active_tab)
                };
                let mut map = self.groups[ix].node.clone();
                if !map.contains_key("id") {
                    map.shift_insert(0, "id".into(), json!(leaf_id()));
                }
                map.insert("type".into(), json!("tabs"));
                let leaves: Vec<Value> = tabs.iter().map(leaf_json).collect();
                if active {
                    *active_id = leaves
                        .get(active_tab)
                        .and_then(|l| l.get("id"))
                        .cloned()
                        .unwrap_or(Value::Null);
                }
                map.shift_remove("children");
                map.shift_remove("currentTab");
                map.insert("children".into(), Value::Array(leaves));
                if active_tab > 0 {
                    map.insert("currentTab".into(), json!(active_tab));
                }
                Value::Object(map)
            }
            Layout::Split {
                direction,
                children,
                node,
            } => {
                let mut map = node.clone();
                if !map.contains_key("id") {
                    map.shift_insert(0, "id".into(), json!(leaf_id()));
                }
                map.insert("type".into(), json!("split"));
                map.shift_remove("children");
                map.shift_remove("direction");
                map.insert(
                    "children".into(),
                    Value::Array(
                        children
                            .iter()
                            .map(|c| self.layout_node_json(c, active_id))
                            .collect(),
                    ),
                );
                map.insert("direction".into(), json!(direction.as_str()));
                Value::Object(map)
            }
        }
    }

    /// Writes `workspace.json` back if the session changed the layout.
    /// Called on the way out, where Obsidian writes it.
    pub fn save_layout(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let fresh = self.layout_json(cx);
        if self.layout.as_ref() == Some(&fresh) {
            return;
        }
        if let Err(err) = state.vault().write_config("workspace.json", &fresh) {
            self.report(format!("workspace.json could not be written: {err}"), cx);
            return;
        }
        self.layout = Some(fresh);
    }

    /// Everything that has to reach the disk before the app goes away: the
    /// note's unsaved edits and the layout.
    pub fn on_quit(&mut self, cx: &mut Context<Self>) {
        self.flush_save(cx);
        if let Some(path) = self.current.clone() {
            self.save_folds(&path, cx);
        }
        self.save_layout(cx);
    }

    // ----- the control socket ----------------------------------------------------

    /// The whole shell as JSON, for the control socket's `state` method: a
    /// test driving the app over the socket reads this instead of a
    /// screenshot. Every key is always there, `null` when there is nothing
    /// to say, so a caller can index into it without guessing. Each view
    /// describes itself; the rest is the shell's own.
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
        // The tab groups and how they are split; `tabs` above is the active
        // group's, as it was before there could be more than one.
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
                RightTab::Tags => "tags",
                RightTab::Properties => "properties",
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

        // Whether the graph is on screen is the shell's to say; the view
        // itself only knows what it would draw.
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
            "status": self.status_text(),
            "status_bar": self.status_bar_text(cx),
            "status_bar_items": self.status_bar_items(cx),
            // The empty tab's pills, by the text they carry, while that is
            // what the main area shows.
            "empty_tab": (self.current.is_none() && self.main_view(cx) == MainView::Empty).then(|| {
                empty_tab_actions(&cx.global::<Commands>().0)
                    .iter()
                    .map(EmptyAction::text)
                    .collect::<Vec<String>>()
            }),
            // The view header's title while it is being renamed in place,
            // so a driver can see the typed name before Enter commits it.
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
    fn describe_modal(&self, rows: usize, cx: &App) -> Value {
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
        // The four list modals fill these in; with no modal up they stay
        // empty rather than disappearing.
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
        // Settings has tabs rather than rows; the tab it is on is the part
        // a caller steers by.
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
    fn typed_text_target(focus: &str, painted: MainView) -> &'static str {
        match focus {
            // A property row or a table cell is the editor's own field.
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
                // `type_text` already routes to an open property row or
                // table cell rather than the body.
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
            // Nothing else is reachable from here, so nothing else is
            // typed into: a focused input is answered by the caller, which
            // asked where the text would go before handing it over.
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
            // An open property row or table cell takes the editor's keys
            // without taking the focus.
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
        // The view header's inline rename field, which takes typed text
        // through the workspace's own input handler.
        if self.title_focus.is_focused(window) {
            return "view-title";
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
        // Both find fields sit in one key context, so the stack cannot tell
        // them apart; the editor holds the two handles and can.
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
        // The tree is in the left dock, not the main area: showing the
        // dock's Files tab is what puts its handle on screen.
        if name == "explorer" {
            self.leave_search_field(window, cx);
            self.left_visible = true;
            self.left_tab = LeftTab::Files;
            self.explorer.update(cx, |e, cx| e.focus_tree(window, cx));
            cx.notify();
            return Ok(());
        }
        match Self::focus_target(name, self.main_view(cx))? {
            // The root div paints in every frame, so its handle is always
            // somewhere the keyboard can sit.
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
    fn focus_target(name: &str, painted: MainView) -> Result<Option<MainView>, String> {
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
    fn status_items(
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
        let state = self.state.as_ref()?;
        state
            .vault()
            .files()
            .iter()
            .find(|f| f.path.eq_ignore_ascii_case(path))
            .map(|f| f.path.clone())
    }

    /// The open vault's files, for the socket's `list_files`: the same list
    /// the explorer is built from, under `folder` when one is given.
    pub fn list_vault_files(&self, folder: Option<&str>) -> Vec<Value> {
        let Some(state) = self.state.as_ref() else {
            return Vec::new();
        };
        // Vault paths keep their case and compare without it, so a folder
        // stored as `Sub` answers to `sub` as well.
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

    // ----- rendering -------------------------------------------------------------

    /// A sidebar's tab: Obsidian shows the view's icon alone, 16 px in a
    /// 28 by 25 button with a 4 px radius, the active one on a wash.
    fn dock_tab(
        &self,
        label: &'static str,
        icon_name: &'static str,
        active: bool,
        theme: &Theme,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(label)
            .flex_shrink_0()
            .w(px(28.))
            .h(px(25.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .rounded(px(4.))
            .when(active, |d| d.bg(theme.bg_active))
            .hover(|s| s.bg(theme.bg_alt))
            .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
            .child(icon(
                icon_name,
                ICON_SIZE,
                if active { theme.text } else { theme.text_muted },
            ))
    }

    /// A pane's own header button: Obsidian's `nav-action-button`, 28 by
    /// 24 with a 4 px radius, the icon muted until the pointer is over it;
    /// `active` is the wash and accent of an `is-active` one.
    fn nav_button(
        id: &'static str,
        icon_name: &'static str,
        label: &'static str,
        active: bool,
        theme: &Theme,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .flex_shrink_0()
            .w(px(28.))
            .h(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .cursor_pointer()
            .when(active, |d| d.bg(theme.bg_alt))
            .hover(|s| s.bg(theme.bg_alt))
            .tooltip(tooltip(label))
            .child(icon(
                icon_name,
                ICON_SIZE,
                if active {
                    theme.accent
                } else {
                    theme.text_muted
                },
            ))
    }

    /// The Backlinks pane: Obsidian's four header actions (collapse,
    /// more context, sort, filter), the filter field when it is shown,
    /// then the two sections.
    fn render_backlinks_pane(&mut self, theme: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let (collapse_all, extra, show_search) = (
            self.backlink_state.collapse_all,
            self.backlink_state.extra_context,
            self.backlink_state.show_search,
        );
        let sort_menu = self.render_backlink_sort_menu(theme, cx);
        let header = div()
            .flex_shrink_0()
            .h(px(HEADER_HEIGHT))
            .p(px(8.))
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .gap(px(2.))
            .child(
                Self::nav_button(
                    "backlinks-collapse",
                    "list",
                    "Collapse results",
                    collapse_all,
                    theme,
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_backlink_collapse_all(cx))),
            )
            .child(
                Self::nav_button(
                    "backlinks-context",
                    "move-vertical",
                    "Show more context",
                    extra,
                    theme,
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_backlink_context(cx))),
            )
            .child(
                // The menu hangs off this wrapper, so it opens under the
                // button wherever the dock is.
                div()
                    .relative()
                    .child(
                        Self::nav_button(
                            "backlinks-sort",
                            "arrow-up-narrow-wide",
                            "Change sort order",
                            false,
                            theme,
                        )
                        .on_click(cx.listener(
                            |this, event: &ClickEvent, _, cx| {
                                this.sort_menu_at = event.position();
                                this.backlink_state.sort_menu_open =
                                    !this.backlink_state.sort_menu_open;
                                cx.notify();
                            },
                        )),
                    )
                    .children(sort_menu),
            )
            .child(
                Self::nav_button(
                    "backlinks-search",
                    "search",
                    "Show search filter",
                    show_search,
                    theme,
                )
                .on_click(
                    cx.listener(|this, _, window, cx| this.toggle_backlink_search(window, cx)),
                ),
            );
        let query = self.backlink_filter.read(cx).text().to_string();
        let filter = show_search.then(|| {
            div()
                .flex_shrink_0()
                .px_2()
                .pb_2()
                .child(self.backlink_filter.clone())
        });
        let body = self.render_sections(RightTab::Backlinks, &query, theme, cx);
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(header)
            .children(filter)
            .child(body)
            .into_any_element()
    }

    /// The Outgoing links pane has no header of its own: Links, then
    /// Unlinked mentions.
    fn render_outgoing_pane(&mut self, theme: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        self.render_sections(RightTab::Outgoing, "", theme, cx)
    }

    /// "Change sort order" for the Backlinks pane: the six orders in three
    /// pairs, a check against the one in force.
    fn render_backlink_sort_menu(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if !self.backlink_state.sort_menu_open {
            return None;
        }
        let current = self.backlink_state.sort;
        let mut list = div()
            .id("backlinks-sort-menu")
            .flex()
            .flex_col()
            .min_w(px(220.))
            .py_1()
            .bg(theme.bg_secondary)
            .border_1()
            .border_color(theme.border)
            .rounded_md()
            .shadow_lg()
            .text_size(px(13.))
            .text_color(theme.text)
            // Keeps a click on an item from reaching the row under it.
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.backlink_state.sort_menu_open = false;
                cx.notify();
            }));
        for (ix, order) in SortOrder::ALL.into_iter().enumerate() {
            if ix > 0 && ix % 2 == 0 {
                list = list.child(div().my_1().h(px(1.)).bg(theme.border));
            }
            let checked = order == current;
            list = list.child(
                div()
                    .id(("backlinks-sort-item", ix))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.bg_alt))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_backlink_sort(order, cx)))
                    .child(
                        div()
                            .w(px(16.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .children(checked.then(|| icon("check", 14., theme.text_muted))),
                    )
                    .child(order.label()),
            );
        }
        Some(
            deferred(
                anchored()
                    .position(self.sort_menu_at)
                    .anchor(Anchor::TopRight)
                    .snap_to_window_with_margin(px(8.))
                    .child(list),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    /// A pane's sections, drawn the way Obsidian's are: a section row with
    /// its count (no fold glyph; the row itself folds), a collapsible row
    /// per file with a triangle and its count, and the file's matches in
    /// one card with the matched text highlighted. The Links section's rows
    /// stand alone with a glyph for what they point at; the Outgoing pane's
    /// unlinked mentions are one card for the note itself, each match
    /// naming the note it mentions. A match wraps, so this is a scrolling
    /// column rather than a uniform list.
    fn render_sections(
        &mut self,
        which: RightTab,
        query: &str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let outgoing = which == RightTab::Outgoing;
        let sections = if outgoing {
            self.outgoing_sections.clone()
        } else {
            self.backlink_sections.clone()
        };
        let prefix = if outgoing { "outgoing" } else { "backlinks" };
        let current = self.current.clone().unwrap_or_default();
        // Obsidian's `.outgoing-link-pane` is padded 12px 12px 32px; the
        // backlink pane 4px 12px 32px, its nav header carrying the rest.
        let mut list = div()
            .id(prefix)
            .flex()
            .flex_col()
            .flex_1()
            .overflow_y_scroll()
            .px(px(12.))
            .pt(px(if outgoing { 12. } else { 4. }))
            .pb(px(32.));
        let mut ix = 0usize;
        let mut next = || {
            ix += 1;
            ix
        };
        for (s_ix, section) in sections.iter().enumerate() {
            let shown = if outgoing {
                section.clone()
            } else {
                backlinks::filter(section, query)
            };
            let folded = self.section_collapsed(which, s_ix);
            let count: SharedString = if section.capped {
                format!("{}+", shown.count).into()
            } else {
                shown.count.to_string().into()
            };
            // A folded section is drawn faint throughout; an open one's
            // count brightens from faint to muted under the pointer.
            list = list.child(
                div()
                    .id(("pane-section", next()))
                    .group("pane-section")
                    .w_full()
                    .flex_shrink_0()
                    .h(px(25.))
                    .mb(px(2.))
                    .px(px(8.))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(4.))
                    .rounded(px(8.))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.bg_alt))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.toggle_section(which, s_ix, cx)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(FONT_UI_SMALL))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(if folded { theme.text_faint } else { theme.text })
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(section.label),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(FONT_UI_SMALLER))
                            .text_color(theme.text_faint)
                            .when(!folded, |d| {
                                d.group_hover("pane-section", |s| s.text_color(theme.text_muted))
                            })
                            .child(count),
                    ),
            );
            if folded {
                continue;
            }
            // Obsidian's `.search-result-container`: 4px 1px 16px.
            let mut body = div()
                .w_full()
                .flex_shrink_0()
                .flex()
                .flex_col()
                .pt(px(4.))
                .pb(px(16.))
                .px(px(1.));
            if !outgoing && s_ix == 0 && shown.groups.is_empty() {
                body = body.child(
                    div()
                        .pl(px(8.))
                        .mb(px(12.))
                        .text_size(px(FONT_UI_SMALL))
                        .text_color(theme.text_faint)
                        .child("No backlinks found."),
                );
            }
            // The Outgoing pane's unlinked mentions: one card, no title row.
            let one_card = outgoing && s_ix == 1;
            if one_card {
                let mut card = Self::match_card(theme);
                for group in &shown.groups {
                    for m in &group.matches {
                        card = card.child(self.render_match(
                            which,
                            next(),
                            &current,
                            group,
                            m,
                            theme,
                            cx,
                        ));
                    }
                }
                body = body.child(card);
                list = list.child(body);
                continue;
            }
            for group in &shown.groups {
                let key = shown.group_key(group);
                let flat = group.matches.is_empty();
                let collapsed = !flat && self.group_collapsed(which, &key);
                let path = group.path.clone();
                let link = group.link.clone();
                let resolved = path.is_some();
                // The fold triangle (a mentions group) or the link's glyph
                // (a Links row) sits in the row's 24px left gutter.
                let gutter: gpui::AnyElement = if flat {
                    let name = group.icon.unwrap_or("link");
                    div()
                        .w(px(16.))
                        .h(px(17.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .opacity(0.85)
                        .child(icon(name, 14., theme.text_muted))
                        .into_any_element()
                } else {
                    let key = key.clone();
                    div()
                        .id(("pane-chevron", next()))
                        .w(px(16.))
                        .h(px(17.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .opacity(0.85)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_group(which, key.clone(), cx);
                            cx.stop_propagation();
                        }))
                        .child(icon(
                            if collapsed {
                                "chevron-right"
                            } else {
                                "chevron-down"
                            },
                            12.,
                            theme.text_muted,
                        ))
                        .into_any_element()
                };
                let flair: Option<SharedString> =
                    (!flat).then(|| group.matches.len().to_string().into());
                let subtext: Option<SharedString> = group.subtext.clone().map(Into::into);
                let unresolved = flat && !resolved;
                list = list.child(
                    div()
                        .id(("pane-group", next()))
                        .group("pane-group")
                        .w_full()
                        .flex_shrink_0()
                        .min_h(px(25.))
                        .mb(px(2.))
                        .pl(px(4.))
                        .pr(px(8.))
                        .py(px(4.))
                        .flex()
                        .flex_row()
                        .items_start()
                        .gap(px(4.))
                        .rounded(px(8.))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme.bg_alt))
                        .when(unresolved, |d| d.tooltip(tooltip("Not created yet")))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            // A Links row follows the link the way the note
                            // would, subpath included, and creates the note
                            // nothing stands behind; a mentions row opens
                            // the file.
                            if let Some(link) = link.clone() {
                                this.follow_link(LinkTarget::Internal(link), cx);
                            } else if let Some(path) = path.clone() {
                                this.open_file(path, None, cx);
                            }
                        }))
                        .child(gutter)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .text_size(px(FONT_UI_SMALL))
                                        .line_height(px(17.))
                                        .text_color(if flat {
                                            theme.text_muted
                                        } else {
                                            theme.text
                                        })
                                        .when(flat, |d| {
                                            d.group_hover("pane-group", |s| {
                                                s.text_color(theme.text)
                                            })
                                        })
                                        .whitespace_nowrap()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .child(SharedString::from(group.label.clone())),
                                )
                                .children(subtext.map(|sub| {
                                    div()
                                        .text_size(px(11.))
                                        .line_height(px(14.))
                                        .text_color(theme.text_faint)
                                        .whitespace_nowrap()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .child(sub)
                                })),
                        )
                        .children(flair.map(|count| {
                            div()
                                .flex_shrink_0()
                                .pt(px(2.))
                                .text_size(px(FONT_UI_SMALLER))
                                .text_color(theme.text_faint)
                                .child(count)
                        })),
                );
                if flat || collapsed {
                    continue;
                }
                let mut card = Self::match_card(theme);
                for m in &group.matches {
                    card =
                        card.child(self.render_match(which, next(), &current, group, m, theme, cx));
                }
                body = body.child(card);
            }
            list = list.child(body);
        }
        list.into_any_element()
    }

    /// Obsidian's `.search-result-file-matches`: the note's background,
    /// a 4px radius, 4px above and 8px below, no border.
    fn match_card(theme: &Theme) -> gpui::Div {
        div()
            .w_full()
            .flex_shrink_0()
            .mt(px(4.))
            .mb(px(8.))
            .flex()
            .flex_col()
            .rounded(px(4.))
            .bg(theme.bg)
    }

    /// One match in a file's card: its context with the matched text
    /// highlighted; for an unlinked mention, the control that turns the
    /// text into a link: a Link button at the row's right that shows under
    /// the pointer (Backlinks), or a chip under the text naming the note
    /// the mention is of (Outgoing links).
    #[allow(clippy::too_many_arguments)]
    fn render_match(
        &self,
        which: RightTab,
        ix: usize,
        current: &str,
        group: &Group,
        m: &PaneMatch,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let path = group.path.clone();
        let offset = m.offset;
        let highlights: Vec<(Range<usize>, HighlightStyle)> = m
            .ranges
            .iter()
            .filter(|r| r.end <= m.text.len())
            .map(|r| {
                (
                    r.clone(),
                    HighlightStyle {
                        background_color: Some(theme.highlight_bg),
                        color: Some(theme.text),
                        ..HighlightStyle::default()
                    },
                )
            })
            .collect();
        let text = StyledText::new(SharedString::from(m.text.clone())).with_highlights(highlights);
        // Which note the mention is in and which it names depend on the
        // pane: the Backlinks pane lists other notes mentioning this one,
        // the Outgoing pane this note mentioning others.
        let outgoing = which == RightTab::Outgoing;
        let link = m.mention.clone().and_then(|mention| {
            let (source, target) = if outgoing {
                (current.to_string(), m.target.clone()?)
            } else {
                (path.clone()?, current.to_string())
            };
            Some((source, target, mention))
        });
        let chip = link
            .clone()
            .filter(|_| outgoing)
            .map(|(source, target, mention)| {
                let name: SharedString = display_name(target.rsplit('/').next().unwrap_or(&target))
                    .to_string()
                    .into();
                div().mt(px(6.)).flex().flex_row().child(
                    div()
                        .id(("pane-chip", ix))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(4.))
                        .px(px(6.))
                        .py(px(4.))
                        .rounded(px(4.))
                        .bg(theme.border)
                        .cursor_pointer()
                        .tooltip(tooltip("Link this file"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.link_mention(
                                source.clone(),
                                target.clone(),
                                mention.clone(),
                                window,
                                cx,
                            );
                            cx.stop_propagation();
                        }))
                        .child(icon("link", 14., theme.text_faint))
                        .child(
                            div()
                                .text_size(px(FONT_UI_SMALLER))
                                .text_color(theme.text_muted)
                                .child(name),
                        ),
                )
            });
        let button = link.filter(|_| !outgoing).map(|(source, target, mention)| {
            div()
                .id(("pane-link", ix))
                .absolute()
                .top(px(2.))
                .right(px(24.))
                .px(px(8.))
                .py(px(4.))
                .rounded(px(5.))
                .bg(theme.border)
                .text_size(px(FONT_UI_SMALLER))
                .text_color(theme.text)
                .cursor_pointer()
                .invisible()
                .group_hover("pane-match", |s| s.visible())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.link_mention(source.clone(), target.clone(), mention.clone(), window, cx);
                    cx.stop_propagation();
                }))
                .child("Link")
        });
        let mut hover = theme.accent;
        hover.a = 0.33;
        div()
            .id(("pane-match", ix))
            .group("pane-match")
            .relative()
            .w_full()
            .flex_shrink_0()
            .pl(px(12.))
            .pr(px(20.))
            .py(px(8.))
            .flex()
            .flex_col()
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(path) = path.clone() {
                    this.open_file(path, None, cx);
                    if let Some(offset) = offset {
                        this.reveal_offset(offset, cx);
                    }
                }
            }))
            .child(
                div()
                    .text_size(px(FONT_UI_SMALLER))
                    .text_color(theme.text_muted)
                    .line_height(relative(1.3))
                    .child(text),
            )
            .children(chip)
            .children(button)
    }

    // ----- the Tags, All properties and Outline panes ---------------------------

    /// Obsidian's `nav-header`: 40 px, 8 px of padding, the buttons
    /// centred 2 px apart.
    fn pane_header() -> gpui::Div {
        div()
            .flex_shrink_0()
            .h(px(HEADER_HEIGHT))
            .p(px(8.))
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .gap(px(2.))
    }

    /// The filter field of a pane, by pane.
    fn pane_filter(&self, which: RightTab) -> &Entity<TextInput> {
        match which {
            RightTab::Tags => &self.tag_filter,
            RightTab::Properties => &self.property_filter,
            RightTab::Outline => &self.outline_filter,
            _ => &self.backlink_filter,
        }
    }

    fn pane_show_search(&mut self, which: RightTab) -> &mut bool {
        match which {
            RightTab::Tags => &mut self.tag_state.show_search,
            RightTab::Properties => &mut self.property_state.show_search,
            RightTab::Outline => &mut self.outline_state.show_search,
            _ => &mut self.backlink_state.show_search,
        }
    }

    /// "Show search filter": shows the box and puts the keyboard in it, or
    /// hides it again, leaving the query (and the rows) as they were.
    fn toggle_pane_search(&mut self, which: RightTab, window: &mut Window, cx: &mut Context<Self>) {
        let flag = self.pane_show_search(which);
        *flag = !*flag;
        if *flag {
            let handle = self.pane_filter(which).focus_handle(cx);
            window.focus(&handle, cx);
        }
        cx.notify();
    }

    /// Escape in the box: hides it and clears the query, so every row is
    /// back, and the keyboard goes to the note.
    fn hide_pane_search(&mut self, which: RightTab, window: &mut Window, cx: &mut Context<Self>) {
        *self.pane_show_search(which) = false;
        self.pane_filter(which)
            .update(cx, |input, cx| input.set_text("", cx));
        self.focus_main(window, cx);
        cx.notify();
    }

    /// The box's clear button: empties it and keeps the keyboard there.
    fn clear_pane_filter(&mut self, which: RightTab, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.pane_filter(which).clone();
        input.update(cx, |input, cx| input.set_text("", cx));
        window.focus(&input.focus_handle(cx), cx);
        cx.notify();
    }

    fn toggle_tag_hierarchy(&mut self, cx: &mut Context<Self>) {
        self.tag_state.use_hierarchy = !self.tag_state.use_hierarchy;
        // Obsidian's tree comes back with every folder open.
        self.tag_state.collapsed.clear();
        cx.notify();
    }

    /// "Expand all" (whose label never changes in Obsidian): folds every
    /// tag, leaves included, or opens them all again.
    fn toggle_tag_folds(&mut self, cx: &mut Context<Self>) {
        if self.tag_state.collapsed.is_empty() {
            self.tag_state.collapsed = panes::tag_keys(&self.tag_tree).into_iter().collect();
        } else {
            self.tag_state.collapsed.clear();
        }
        cx.notify();
    }

    fn toggle_tag_fold(&mut self, key: String, cx: &mut Context<Self>) {
        if !self.tag_state.collapsed.remove(&key) {
            self.tag_state.collapsed.insert(key);
        }
        cx.notify();
    }

    fn set_pane_sort(&mut self, which: RightTab, order: PaneSort, cx: &mut Context<Self>) {
        match which {
            RightTab::Tags => {
                self.tag_state.sort = order;
                self.tag_state.sort_menu_open = false;
                panes::sort_tags(&mut self.tag_tree, order);
            }
            _ => {
                self.property_state.sort = order;
                self.property_state.sort_menu_open = false;
            }
        }
        cx.notify();
    }

    fn toggle_outline_fold(&mut self, ix: usize, cx: &mut Context<Self>) {
        if !self.outline_state.collapsed.remove(&ix) {
            self.outline_state.collapsed.insert(ix);
        }
        cx.notify();
    }

    /// The headings that have children: what "Collapse all" folds.
    fn outline_parents(&self) -> Vec<usize> {
        let depths = panes::outline_depths(&self.outline);
        (0..depths.len())
            .filter(|&ix| depths.get(ix + 1).is_some_and(|&d| d > depths[ix]))
            .collect()
    }

    /// The top-level headings with children: what "Collapse all" folds,
    /// leaving the folds under them as they were, and what "Expand all"
    /// counts.
    fn outline_top_parents(&self) -> Vec<usize> {
        let depths = panes::outline_depths(&self.outline);
        self.outline_parents()
            .into_iter()
            .filter(|&ix| depths[ix] == 0)
            .collect()
    }

    /// Whether every top-level heading with children is folded, which is
    /// when the button reads "Expand all".
    fn outline_all_folded(&self) -> bool {
        let parents = self.outline_top_parents();
        !parents.is_empty()
            && parents
                .iter()
                .all(|ix| self.outline_state.collapsed.contains(ix))
    }

    /// "Collapse all" folds the top-level headings and leaves the deeper
    /// folds alone; "Expand all" opens everything.
    fn toggle_outline_folds(&mut self, cx: &mut Context<Self>) {
        if self.outline_all_folded() {
            self.outline_state.collapsed.clear();
        } else {
            let top = self.outline_top_parents();
            self.outline_state.collapsed.extend(top);
        }
        cx.notify();
    }

    fn toggle_follow_cursor(&mut self, cx: &mut Context<Self>) {
        self.outline_state.follow_cursor = !self.outline_state.follow_cursor;
        self.outline_state.scroll_pending = true;
        cx.notify();
    }

    /// An Outline row's click: in an editing mode the cursor goes to the
    /// heading's line, column 0, scrolled to the vertical centre, and the
    /// editor takes the keyboard; in reading mode the heading scrolls to
    /// the top edge.
    fn reveal_heading(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.reading
            .update(cx, |r, cx| r.scroll_to_offset(offset, cx));
        if self.mode.is_editing() {
            self.editor
                .update(cx, |e, cx| e.reveal_offset_centered(offset, cx));
        } else {
            self.editor.update(cx, |e, cx| e.reveal_offset(offset, cx));
        }
        self.request_focus_main(cx);
    }

    /// The heading whose section holds the cursor (editing) or the block at
    /// the top of the view (reading); none above the first heading.
    fn current_heading(&self, cx: &App) -> Option<usize> {
        let path = self.current.as_deref()?;
        let offset = match self.mode {
            ViewMode::Reading => self.reading.read(cx).top_offset()?,
            mode if mode.is_editing() => {
                let editor = self.editor.read(cx);
                if editor.name.as_ref() != path {
                    return None;
                }
                editor.selection().start
            }
            _ => return None,
        };
        panes::heading_at(&self.outline, offset)
    }

    /// Brings the Outline's mark up to date when the cursor or the reading
    /// view moved, and asks for a draw only when the mark moved.
    fn follow_heading(&mut self, cx: &mut Context<Self>) {
        if !self.right_visible || self.right_tab != RightTab::Outline {
            return;
        }
        let active = self.current_heading(cx);
        if active != self.outline_state.active {
            self.outline_state.active = active;
            self.outline_state.scroll_pending = true;
            cx.notify();
        }
    }

    /// The Tags pane's rows as drawn now.
    fn tag_rows(&self, cx: &App) -> Vec<panes::TagRow> {
        panes::tag_rows(
            &self.tag_tree,
            self.tag_state.sort,
            self.tag_state.use_hierarchy,
            &self.tag_state.collapsed,
            self.tag_filter.read(cx).text(),
        )
    }

    fn property_rows(&self, cx: &App) -> Vec<panes::PropertyRow> {
        panes::property_rows(
            &self.property_entries,
            self.property_state.sort,
            self.property_filter.read(cx).text(),
        )
    }

    fn outline_rows(&self, cx: &App) -> Vec<panes::OutlineRow> {
        panes::outline_rows(
            &self.outline,
            &self.outline_state.collapsed,
            self.outline_filter.read(cx).text(),
            self.outline_state.active,
        )
    }

    /// The search box under a pane's header: Obsidian's
    /// `search-input-container`, 30 px tall with 4 px above and below it
    /// inside the header, so the rows move down 38 px, and a round clear
    /// button at its right end once there is text.
    fn render_pane_filter(
        &self,
        which: RightTab,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let input = self.pane_filter(which).clone();
        let has_text = !input.read(cx).text().is_empty();
        // Obsidian's `search-input-container`: the field's text starts
        // 30 px in, behind a magnifier at the left.
        div().flex_shrink_0().px(px(8.)).pb(px(8.)).child(
            div()
                .relative()
                .w_full()
                .h(px(30.))
                .child(input)
                .child(
                    div()
                        .absolute()
                        .left(px(8.))
                        .top(px(0.))
                        .h(px(30.))
                        .flex()
                        .items_center()
                        .child(icon("search", 14., theme.text_muted)),
                )
                .children(has_text.then(|| {
                    div()
                        .id(("pane-filter-clear", which as usize))
                        .absolute()
                        .right(px(2.))
                        .top(px(0.))
                        .w(px(28.))
                        .h(px(30.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .rounded_full()
                        .tooltip(tooltip("Clear search"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.clear_pane_filter(which, window, cx)
                        }))
                        .child(icon("x", 14., theme.text_muted))
                })),
        )
    }

    /// "Change sort order" for the Tags and All properties panes: the two
    /// name orders, a separator, the two frequency orders, a check against
    /// the one in force.
    fn render_pane_sort_menu(
        &mut self,
        which: RightTab,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let (open, current, noun, id) = match which {
            RightTab::Tags => (
                self.tag_state.sort_menu_open,
                self.tag_state.sort,
                "Tag name",
                "tags-sort-menu",
            ),
            _ => (
                self.property_state.sort_menu_open,
                self.property_state.sort,
                "Property name",
                "properties-sort-menu",
            ),
        };
        if !open {
            return None;
        }
        let mut list = div()
            .id(id)
            .flex()
            .flex_col()
            .min_w(px(196.))
            .py_1()
            .bg(theme.bg_secondary)
            .border_1()
            .border_color(theme.border)
            .rounded_md()
            .shadow_lg()
            .text_size(px(13.))
            .text_color(theme.text)
            .occlude()
            .on_mouse_down_out(cx.listener(move |this, _, _, cx| {
                match which {
                    RightTab::Tags => this.tag_state.sort_menu_open = false,
                    _ => this.property_state.sort_menu_open = false,
                }
                cx.notify();
            }));
        for (ix, order) in PaneSort::ALL.into_iter().enumerate() {
            if ix == 2 {
                list = list.child(div().my_1().h(px(1.)).bg(theme.border));
            }
            let checked = order == current;
            list = list.child(
                div()
                    .id((id, ix))
                    .h(px(25.))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.bg_alt))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.set_pane_sort(which, order, cx)),
                    )
                    .child(
                        div()
                            .w(px(16.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .children(checked.then(|| icon("check", 14., theme.text_muted))),
                    )
                    .child(SharedString::from(order.label(noun))),
            );
        }
        Some(
            deferred(
                anchored()
                    .position(self.sort_menu_at)
                    .anchor(Anchor::TopRight)
                    .snap_to_window_with_margin(px(8.))
                    .child(list),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    /// Obsidian's `tree-item-self`: 25 px tall at a 27 px pitch,
    /// `padding: 4px 8px 4px 24px` with 17 px more on the left per level of
    /// depth, an 8 px radius, 13 px muted text that brightens under the
    /// pointer (or when `active`, the Outline's `is-active` mark), a faint
    /// flair at the right edge, a fold triangle or an icon in the gutter
    /// 20 px before the text, and an indentation guide for every level.
    /// A `wrap` label wraps at 17 px per line; the others are cut with an
    /// ellipsis.
    #[allow(clippy::too_many_arguments)]
    fn tree_row(
        id: (&'static str, usize),
        depth: usize,
        fold: Option<(bool, FoldAction)>,
        icon_name: Option<&'static str>,
        label: gpui::AnyElement,
        flair: Option<SharedString>,
        active: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let text_x = 24. + 17. * depth as f32;
        let gutter = match (fold, icon_name) {
            (Some((collapsed, on_fold)), _) => Some(
                div()
                    .id((id.0, id.1 + 1_000_000))
                    .absolute()
                    .left(px(text_x - 20.))
                    .top(px(4.))
                    .w(px(16.))
                    .h(px(17.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .opacity(0.85)
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        on_fold(this, cx);
                        cx.stop_propagation();
                    }))
                    .child(
                        div()
                            .text_color(theme.text_faint)
                            .group_hover("tree-row", |s| s.text_color(theme.text_muted))
                            .child(icon(
                                if collapsed {
                                    "chevron-right"
                                } else {
                                    "chevron-down"
                                },
                                10.,
                                theme.text_faint,
                            )),
                    ),
            ),
            (None, Some(name)) => Some(
                div()
                    .id((id.0, id.1 + 1_000_000))
                    .absolute()
                    .left(px(text_x - 20.))
                    .top(px(4.))
                    .w(px(16.))
                    .h(px(17.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .opacity(0.85)
                    .child(icon(name, 14., theme.text_muted)),
            ),
            (None, None) => None,
        };
        div()
            .id(id)
            .group("tree-row")
            .relative()
            .w_full()
            .flex_shrink_0()
            .min_h(px(25.))
            .mb(px(2.))
            .pl(px(text_x))
            .pr(px(8.))
            .py(px(4.))
            .flex()
            .flex_row()
            .items_start()
            .rounded(px(8.))
            .cursor_pointer()
            .when(active, |d| d.bg(theme.bg_alt))
            .hover(|s| s.bg(theme.bg_alt))
            // Obsidian's `tree-item-children` guides: a hairline 12 px in
            // from each ancestor's text, the row's full pitch tall.
            .children((1..=depth).map(|k| {
                div()
                    .absolute()
                    .left(px(17. * k as f32 - 5.))
                    .top(px(0.))
                    .bottom(px(-2.))
                    .w(px(1.))
                    .bg(theme.border)
            }))
            .children(gutter)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(FONT_UI_SMALL))
                    .line_height(px(17.))
                    .text_color(if active { theme.text } else { theme.text_muted })
                    .group_hover("tree-row", |s| s.text_color(theme.text))
                    .child(label),
            )
            .children(flair.map(|count| {
                div()
                    .flex_shrink_0()
                    .pl(px(4.))
                    .pt(px(3.))
                    .text_size(px(FONT_UI_SMALLER))
                    .line_height(px(12.))
                    .text_color(theme.text_faint)
                    .group_hover("tree-row", |s| s.text_color(theme.text_muted))
                    .child(count)
            }))
    }

    /// A one-line label cut with an ellipsis.
    fn tree_label(text: impl Into<SharedString>) -> gpui::AnyElement {
        div()
            .whitespace_nowrap()
            .overflow_hidden()
            .text_ellipsis()
            .child(text.into())
            .into_any_element()
    }

    /// The Tags pane: sort, nested, expand-all and search buttons, the
    /// filter when shown, then the tags as a tree (or a flat list) with
    /// their counts; a row's click searches `tag:#path`.
    fn render_tags_pane(&mut self, theme: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let (hierarchy, show_search) = (self.tag_state.use_hierarchy, self.tag_state.show_search);
        let sort_menu = self.render_pane_sort_menu(RightTab::Tags, theme, cx);
        let header = Self::pane_header()
            .child(
                div()
                    .relative()
                    .child(
                        Self::nav_button(
                            "tags-sort",
                            "arrow-up-narrow-wide",
                            "Change sort order",
                            false,
                            theme,
                        )
                        .on_click(cx.listener(
                            |this, event: &ClickEvent, _, cx| {
                                this.sort_menu_at = event.position();
                                this.tag_state.sort_menu_open = !this.tag_state.sort_menu_open;
                                cx.notify();
                            },
                        )),
                    )
                    .children(sort_menu),
            )
            .child(
                Self::nav_button(
                    "tags-nested",
                    "folder-tree",
                    "Show nested tags",
                    hierarchy,
                    theme,
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_tag_hierarchy(cx))),
            )
            .child(
                Self::nav_button(
                    "tags-expand",
                    "chevrons-up-down",
                    "Expand all",
                    false,
                    theme,
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_tag_folds(cx))),
            )
            .child(
                Self::nav_button(
                    "tags-search",
                    "search",
                    "Show search filter",
                    show_search,
                    theme,
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.toggle_pane_search(RightTab::Tags, window, cx)
                })),
            );
        let filter = show_search.then(|| self.render_pane_filter(RightTab::Tags, theme, cx));
        let rows = self.tag_rows(cx);
        // Obsidian's `.tag-container`: padding 12px 12px 32px.
        let mut list = div()
            .id("tags")
            .flex()
            .flex_col()
            .flex_1()
            .overflow_y_scroll()
            .px(px(12.))
            .pt(px(12.))
            .pb(px(32.));
        for (ix, row) in rows.into_iter().enumerate() {
            let path = row.path.clone();
            let fold = row.has_children.then(|| {
                let key = row.key.clone();
                let on_fold: FoldAction =
                    Box::new(move |this, cx| this.toggle_tag_fold(key.clone(), cx));
                (row.collapsed, on_fold)
            });
            // The parent part shows only in flat mode, in the same colour.
            let label = if hierarchy {
                row.name.clone()
            } else {
                row.path.clone()
            };
            list = list.child(
                Self::tree_row(
                    ("tag-row", ix),
                    row.depth,
                    fold,
                    None,
                    Self::tree_label(label),
                    Some(row.count.to_string().into()),
                    false,
                    theme,
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.show_search(&format!("tag:#{path}"), Some(window), cx);
                })),
            );
        }
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(header)
            .children(filter)
            .child(list)
            .into_any_element()
    }

    /// The All properties pane: sort and search buttons, the filter when
    /// shown, then one row per key with its widget's icon and the notes
    /// carrying it; a row's click searches `["key"]`.
    fn render_properties_pane(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let show_search = self.property_state.show_search;
        let sort_menu = self.render_pane_sort_menu(RightTab::Properties, theme, cx);
        let header = Self::pane_header()
            .child(
                div()
                    .relative()
                    .child(
                        Self::nav_button(
                            "properties-sort",
                            "arrow-up-narrow-wide",
                            "Change sort order",
                            false,
                            theme,
                        )
                        .on_click(cx.listener(
                            |this, event: &ClickEvent, _, cx| {
                                this.sort_menu_at = event.position();
                                this.property_state.sort_menu_open =
                                    !this.property_state.sort_menu_open;
                                cx.notify();
                            },
                        )),
                    )
                    .children(sort_menu),
            )
            .child(
                Self::nav_button(
                    "properties-search",
                    "search",
                    "Show search filter",
                    show_search,
                    theme,
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.toggle_pane_search(RightTab::Properties, window, cx)
                })),
            );
        let filter = show_search.then(|| self.render_pane_filter(RightTab::Properties, theme, cx));
        let rows = self.property_rows(cx);
        // Obsidian's `.view-content`: padding 4px 12px 32px.
        let mut list = div()
            .id("properties")
            .flex()
            .flex_col()
            .flex_1()
            .overflow_y_scroll()
            .px(px(12.))
            .pt(px(4.))
            .pb(px(32.));
        for (ix, row) in rows.into_iter().enumerate() {
            let name = row.name.clone();
            list = list.child(
                Self::tree_row(
                    ("property-row", ix),
                    0,
                    None,
                    Some(row.icon),
                    Self::tree_label(row.name.clone()),
                    Some(row.count.to_string().into()),
                    false,
                    theme,
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.show_search(&format!("[\"{name}\"]"), Some(window), cx);
                })),
            );
        }
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(header)
            .children(filter)
            .child(list)
            .into_any_element()
    }

    /// The Outline pane: search, auto-scroll and collapse-all buttons, the
    /// filter when shown, then the headings as a structural tree with the
    /// one holding the cursor marked, or "No headings found.".
    fn render_outline_pane(&mut self, theme: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        // The mark follows the cursor and the reading view; a mode or note
        // switch lands here before anything else can notice it.
        let active = self.current_heading(cx);
        if active != self.outline_state.active {
            self.outline_state.active = active;
            self.outline_state.scroll_pending = true;
        }
        let (follow, show_search) = (
            self.outline_state.follow_cursor,
            self.outline_state.show_search,
        );
        let all_folded = self.outline_all_folded();
        let header = Self::pane_header()
            .child(
                Self::nav_button(
                    "outline-search",
                    "search",
                    "Show search filter",
                    show_search,
                    theme,
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.toggle_pane_search(RightTab::Outline, window, cx)
                })),
            )
            .child(
                Self::nav_button(
                    "outline-follow",
                    "gallery-vertical",
                    "Auto-scroll to current section",
                    follow,
                    theme,
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_follow_cursor(cx))),
            )
            .child(
                if all_folded {
                    Self::nav_button(
                        "outline-fold",
                        "chevrons-up-down",
                        "Expand all",
                        false,
                        theme,
                    )
                } else {
                    Self::nav_button(
                        "outline-fold",
                        "chevrons-down-up",
                        "Collapse all",
                        false,
                        theme,
                    )
                }
                .on_click(cx.listener(|this, _, _, cx| this.toggle_outline_folds(cx))),
            );
        let filter = show_search.then(|| self.render_pane_filter(RightTab::Outline, theme, cx));
        let body: gpui::AnyElement = if self.outline.is_empty() {
            // Obsidian's `.pane-empty`: 13 px faint, padding 24px 16px,
            // margin 8px 0, centred.
            div()
                .flex_1()
                .px(px(12.))
                .child(
                    div()
                        .my(px(8.))
                        .px(px(16.))
                        .py(px(24.))
                        .text_size(px(FONT_UI_SMALL))
                        .text_color(theme.text_faint)
                        .text_center()
                        .child("No headings found."),
                )
                .into_any_element()
        } else {
            let rows = self.outline_rows(cx);
            if follow && self.outline_state.scroll_pending {
                self.outline_state.scroll_pending = match rows.iter().position(|r| r.active) {
                    Some(ix) => !scroll_outline_to(&self.outline_state.scroll, ix),
                    None => false,
                };
            }
            let mut list = div()
                .id("outline")
                .flex()
                .flex_col()
                .flex_1()
                .overflow_y_scroll()
                .track_scroll(&self.outline_state.scroll)
                .px(px(12.))
                .pt(px(4.))
                .pb(px(32.));
            for row in rows {
                let offset = row.offset;
                let fold = row.has_children.then(|| {
                    let ix = row.ix;
                    let on_fold: FoldAction =
                        Box::new(move |this, cx| this.toggle_outline_fold(ix, cx));
                    (row.collapsed, on_fold)
                });
                // The text wraps; a search match is Obsidian's
                // `suggestion-highlight`, drawn in the accent.
                let highlights: Vec<(Range<usize>, HighlightStyle)> = row
                    .highlights
                    .iter()
                    .filter(|r| r.end <= row.text.len())
                    .map(|r| {
                        (
                            r.clone(),
                            HighlightStyle {
                                color: Some(theme.accent),
                                ..HighlightStyle::default()
                            },
                        )
                    })
                    .collect();
                let label = StyledText::new(SharedString::from(row.text.clone()))
                    .with_highlights(highlights)
                    .into_any_element();
                list = list.child(
                    Self::tree_row(
                        ("outline-row", row.ix),
                        row.depth,
                        fold,
                        None,
                        label,
                        None,
                        row.active,
                        theme,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.reveal_heading(offset, cx))),
                );
            }
            list.into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(header)
            .children(filter)
            .child(body)
            .into_any_element()
    }

    /// Whether a left-dock pane's core plugin is on, by its id.
    fn left_pane_enabled(&self, id: &str) -> bool {
        self.state
            .as_ref()
            .is_none_or(|state| state.core_plugin_enabled(id))
    }

    /// Whether a right-dock pane's core plugin is on. Each of the five is
    /// its own plugin in `core-plugins.json`, and a vault with no file has
    /// them all, so the default is what `CORE_PLUGINS` says.
    fn pane_enabled(&self, tab: RightTab) -> bool {
        let id = match tab {
            RightTab::Backlinks => "backlink",
            RightTab::Outgoing => "outgoing-link",
            RightTab::Tags => "tag-pane",
            RightTab::Properties => "properties",
            RightTab::Outline => "outline",
        };
        self.state
            .as_ref()
            .is_none_or(|state| state.core_plugin_enabled(id))
    }

    fn render_right_dock(&mut self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        // The pane whose plugin was just turned off has no tab any more,
        // so it cannot be the one on screen either: the first that is
        // still on takes over, as Obsidian moves to a neighbouring leaf.
        if !self.pane_enabled(self.right_tab) {
            if let Some(next) = [
                RightTab::Backlinks,
                RightTab::Outgoing,
                RightTab::Tags,
                RightTab::Properties,
                RightTab::Outline,
            ]
            .into_iter()
            .find(|t| self.pane_enabled(*t))
            {
                self.right_tab = next;
            }
        }
        let tab = self.right_tab;
        let body: gpui::AnyElement = match tab {
            RightTab::Backlinks => self.render_backlinks_pane(theme, cx),
            RightTab::Outgoing => self.render_outgoing_pane(theme, cx),
            RightTab::Outline => self.render_outline_pane(theme, cx),
            RightTab::Tags => self.render_tags_pane(theme, cx),
            RightTab::Properties => self.render_properties_pane(theme, cx),
        };
        // A pane whose core plugin is off is not offered: Obsidian takes
        // the view away with the plugin.
        let tab_button = |this: &mut Self,
                          label: &'static str,
                          icon_name: &'static str,
                          which: RightTab,
                          cx: &mut Context<Self>| {
            this.pane_enabled(which).then(|| {
                this.dock_tab(
                    label,
                    icon_name,
                    tab == which,
                    theme,
                    move |this, _, cx| this.select_right_tab(which, cx),
                    cx,
                )
            })
        };
        let frameless = self.frameless;
        let mac = self.mac_frame();
        // Obsidian's tab order and icon names (`links-coming-in` and
        // `links-going-out` are its own drawings; Silica's are in `assets`).
        div()
            .relative()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .w(px(self.right_width as f32))
            .h_full()
            .bg(theme.bg_secondary)
            .map(|d| self.corners.right().apply(d))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .h(px(HEADER_HEIGHT))
                    .pl(px(8.))
                    .pr(if frameless && !mac {
                        px(3. * RIBBON_WIDTH)
                    } else {
                        px(8.)
                    })
                    .gap(px(3.))
                    .border_b_1()
                    .border_color(theme.border)
                    .children(tab_button(
                        self,
                        "Backlinks",
                        "links-coming-in",
                        RightTab::Backlinks,
                        cx,
                    ))
                    .children(tab_button(
                        self,
                        "Outgoing links",
                        "links-going-out",
                        RightTab::Outgoing,
                        cx,
                    ))
                    .children(tab_button(self, "Tags", "tags", RightTab::Tags, cx))
                    .children(tab_button(
                        self,
                        "All properties",
                        "archive",
                        RightTab::Properties,
                        cx,
                    ))
                    .children(tab_button(self, "Outline", "list", RightTab::Outline, cx))
                    .child(window_handle(
                        div().id("right-dock-space").flex_1().h_full(),
                        frameless,
                    ))
                    // The window's top-right corner on macOS: the right
                    // sidebar's toggle, which Obsidian fixes there.
                    .when(mac, |d| d.child(self.sidebar_toggle(false, theme, cx))),
            )
            .child(div().flex_1().overflow_hidden().child(body))
            .child(self.dock_handle(false, theme))
    }
}

/// Unlinked mentions for `path`: other notes mentioning its names (incoming)
/// and names of other notes mentioned in it (outgoing).
/// Whether the status bar carries the parse, span and shape numbers.
///
/// They are Silica's own: Obsidian's status bar has no diagnostics, so
/// they are off unless `SILICA_DEBUG_STATUS` is set, and read once rather
/// than on every frame.
fn debug_status() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("SILICA_DEBUG_STATUS").is_some())
}

/// The `s` a count needs, for the notices that carry one.
fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

fn mention_sections(
    state: &VaultState,
    path: &str,
    finder: Option<&Finder>,
    extra: bool,
) -> MentionSections {
    let texts = state.texts();
    let cache = state.cache();
    let base = path
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .trim_end_matches(".md")
        .to_string();
    let mut own: Vec<String> = vec![base];
    if let Some(meta) = cache.file(path) {
        own.extend(frontmatter_aliases(meta));
    }
    let own_refs: Vec<Name<'_>> = own.iter().map(|t| Name { text: t, path }).collect();
    let own_finder = Finder::new(&own_refs);
    let incoming = backlinks::unlinked_incoming(&texts, path, own_finder.as_ref(), extra);
    let outgoing = match texts.get(path) {
        Some(text) => backlinks::unlinked_outgoing(text, path, finder, extra),
        None => Section::empty(backlinks::UNLINKED),
    };
    MentionSections {
        path: path.to_string(),
        incoming,
        outgoing,
    }
}

/// An absolute path inside `root` for a vault-relative one, refusing
/// everything that points outside the vault: the folder is the only thing
/// Silica may write, and one socket line must not be able to aim a write
/// out of it.
///
/// The leaf usually does not exist yet - `write_file` is what creates it -
/// so the check cannot be a `canonicalize` of the whole path. It is the
/// deepest ancestor that exists at all that has to resolve inside the
/// vault, because a symlinked parent is how a path that is not there yet
/// still leaves. `symlink_metadata` counts a dangling link as existing: a
/// write through one creates the file at its target, wherever that is, and
/// nothing is left to resolve, so it is refused.
fn vault_path_in(root: &Path, path: &str) -> Result<PathBuf, String> {
    let mut absolute = root.to_path_buf();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(part) => absolute.push(part),
            Component::CurDir => {}
            Component::ParentDir => return Err(format!("{path} leaves the vault")),
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!("{path} is not a vault-relative path"))
            }
        }
    }
    if absolute.as_path() == root {
        return Err(format!("{path} names the vault folder, not a file"));
    }
    let real_root = root
        .canonicalize()
        .map_err(|error| format!("{}: {error}", root.display()))?;
    let mut existing: &Path = absolute.as_path();
    let inside = loop {
        if existing.symlink_metadata().is_ok() {
            break match existing.canonicalize() {
                Ok(real) => real.starts_with(&real_root),
                // A dangling link has nothing to resolve; so has a path the
                // process may not look at. Neither can be shown to stay in
                // the vault, so neither is written through.
                Err(_) => false,
            };
        }
        match existing.parent() {
            // The walk stops at `root` at the latest: it canonicalized
            // above, so it exists.
            Some(parent) if parent.starts_with(root) => existing = parent,
            _ => break false,
        }
    };
    if !inside {
        return Err(format!("{path} leaves the vault"));
    }
    Ok(absolute)
}

/// Inserts `list` under `key`, capped to `rows`, with a sibling
/// `{key}_total`. The total is always there, `null` when the cap dropped
/// nothing, so a caller has one rule for every list in the dump.
fn put_capped(into: &mut Map<String, Value>, key: &str, list: Vec<Value>, rows: usize) {
    let total = list.len();
    into.insert(
        key.to_string(),
        Value::Array(list.into_iter().take(rows).collect()),
    );
    into.insert(
        format!("{key}_total"),
        if total > rows {
            json!(total)
        } else {
            Value::Null
        },
    );
}

/// An Outline row as drawn, with the offset the pane scrolls the note to.
fn outline_json(row: &panes::OutlineRow) -> Value {
    json!({
        "level": row.level,
        "text": row.text,
        "offset": row.offset,
        "line": row.line,
        "ix": row.ix,
        "depth": row.depth,
        "has_children": row.has_children,
        "collapsed": row.collapsed,
        "active": row.active,
        "highlights": row.highlights.iter().map(|r| json!([r.start, r.end])).collect::<Vec<_>>(),
    })
}

/// A Backlinks or Outgoing links pane flattened to the rows it draws: a
/// section, then each of its files, then each file's matches.
fn section_rows_json(
    sections: &[Section],
    section_folded: &dyn Fn(usize) -> bool,
    group_folded: &dyn Fn(&str) -> bool,
) -> Vec<Value> {
    let mut out = Vec::new();
    let span = |r: &Range<usize>| json!([r.start, r.end]);
    for (ix, section) in sections.iter().enumerate() {
        out.push(json!({
            "kind": "section",
            "label": section.label,
            "count": section.count,
            "capped": section.capped,
            "collapsed": section_folded(ix),
        }));
        for group in &section.groups {
            out.push(json!({
                "kind": "file",
                "label": group.label,
                "path": group.path,
                "link": group.link,
                "subtext": group.subtext,
                "icon": group.icon,
                "count": group.matches.len(),
                "collapsed": !group.matches.is_empty()
                    && group_folded(&section.group_key(group)),
            }));
            for m in &group.matches {
                out.push(json!({
                    "kind": "match",
                    "path": group.path,
                    "text": m.text,
                    "ranges": m.ranges.iter().map(span).collect::<Vec<_>>(),
                    "offset": m.offset,
                    "mention": m.mention.as_ref().map(span),
                    "target": m.target,
                }));
            }
        }
    }
    out
}

/// A tag row as drawn: `name` keeps the `#` and the full path that earlier
/// dumps had; the pane itself paints neither.
fn tag_json(row: &panes::TagRow) -> Value {
    json!({
        "name": format!("#{}", row.path),
        "path": row.path,
        "parent": row.parent,
        "label": row.name,
        "count": row.count,
        "depth": row.depth,
        "has_children": row.has_children,
        "collapsed": row.collapsed,
    })
}

/// The key contexts of the focused element, outermost first: the names
/// `key_context` puts on each view, which is what a hotkey is matched
/// against.
pub(crate) fn context_names(window: &Window) -> Vec<String> {
    window
        .context_stack()
        .iter()
        .filter_map(|c| c.primary().map(|e| e.key.to_string()))
        .collect()
}

/// Removes `path`, and anything beneath it when it is a folder, from a
/// navigation history, answering where the one-based cursor lands. Entries
/// before the cursor carry it back with them, so back and forward still
/// reach the same notes they did.
fn prune_history(history: &mut Vec<HistoryEntry>, ix: usize, path: &str) -> usize {
    let prefix = format!("{path}/");
    let mut cursor = ix;
    let mut seen = 0;
    history.retain(|entry| {
        let keep = entry.path != path && !entry.path.starts_with(&prefix);
        if !keep && seen < ix {
            cursor -= 1;
        }
        seen += 1;
        keep
    });
    cursor.min(history.len())
}

/// A `tabs` node of `workspace.json` read into tabs, before any group
/// exists for it: each leaf's id with its tab, the `currentTab`, and the
/// node's own keys less `children` and `currentTab`, which are rewritten.
#[derive(Debug, Clone, PartialEq)]
struct GroupSpec {
    tabs: Vec<(String, Tab)>,
    active_tab: usize,
    node: Map<String, Value>,
}

/// `main` as read: the tree of splits over the groups, without the groups
/// existing yet. Pure, so the reading can be tested without a window.
#[derive(Debug, Clone, PartialEq)]
enum LayoutSpec {
    Group(GroupSpec),
    Split {
        direction: SplitDirection,
        children: Vec<LayoutSpec>,
        node: Map<String, Value>,
    },
}

/// Reads `main`. A `tabs` node with no leaf left (every file gone) is
/// dropped, and a split with no child left goes with its children; a split
/// with one child stays nested as Obsidian keeps it. `known` says whether a
/// leaf's file is still in the vault.
fn read_main(
    node: &Value,
    default_mode: ViewMode,
    known: &dyn Fn(&str) -> bool,
) -> Option<LayoutSpec> {
    let object = node.as_object()?;
    match node.get("type").and_then(Value::as_str) {
        Some("tabs") => {
            let tabs: Vec<(String, Tab)> = node
                .get("children")
                .and_then(Value::as_array)
                .map(|leaves| {
                    leaves
                        .iter()
                        .filter_map(|leaf| tab_from_leaf(leaf, default_mode, known))
                        .collect()
                })
                .unwrap_or_default();
            if tabs.is_empty() {
                return None;
            }
            let active_tab = node
                .get("currentTab")
                .and_then(Value::as_u64)
                .map(|t| t as usize)
                .unwrap_or(0)
                .min(tabs.len() - 1);
            let mut own = object.clone();
            own.shift_remove("children");
            own.shift_remove("currentTab");
            Some(LayoutSpec::Group(GroupSpec {
                tabs,
                active_tab,
                node: own,
            }))
        }
        Some("split") => {
            let children: Vec<LayoutSpec> = node
                .get("children")
                .and_then(Value::as_array)
                .map(|c| {
                    c.iter()
                        .filter_map(|child| read_main(child, default_mode, known))
                        .collect()
                })
                .unwrap_or_default();
            if children.is_empty() {
                return None;
            }
            let direction = SplitDirection::parse(node.get("direction").and_then(Value::as_str));
            let mut own = object.clone();
            own.shift_remove("children");
            own.shift_remove("direction");
            Some(LayoutSpec::Split {
                direction,
                children,
                node: own,
            })
        }
        _ => None,
    }
}

/// The tab a `leaf` node of `workspace.json` stands for, with the leaf's
/// id, or `None` for a view Silica has no tab for or a file that is gone.
fn tab_from_leaf(
    leaf: &Value,
    default_mode: ViewMode,
    known: &dyn Fn(&str) -> bool,
) -> Option<(String, Tab)> {
    let id = leaf.get("id").and_then(Value::as_str)?;
    let view = leaf
        .get("state")
        .and_then(|s| s.get("type"))
        .and_then(Value::as_str);
    let inner = leaf.get("state").and_then(|s| s.get("state"));
    let file = inner.and_then(|s| s.get("file")).and_then(Value::as_str);
    let tab = match (view, file) {
        (_, Some(file)) => {
            if !known(file) {
                return None;
            }
            let mode = match inner.and_then(|s| s.get("mode")).and_then(Value::as_str) {
                Some("preview") => ViewMode::Reading,
                Some("source")
                    if inner
                        .and_then(|s| s.get("source"))
                        .and_then(Value::as_bool)
                        .unwrap_or(false) =>
                {
                    ViewMode::Source
                }
                Some("source") => ViewMode::LivePreview,
                _ => default_mode,
            };
            Tab {
                path: Some(file.to_string()),
                mode,
                history: vec![HistoryEntry::new(file.to_string(), mode)],
                history_ix: 1,
                leaf: Some(leaf.clone()),
                view_state: inner.and_then(|s| s.get("viewState")).cloned(),
            }
        }
        (Some("empty"), None) => Tab {
            leaf: Some(leaf.clone()),
            ..Tab::empty(default_mode)
        },
        (Some("graph"), None) => Tab {
            leaf: Some(leaf.clone()),
            ..Tab::empty(ViewMode::Graph)
        },
        _ => return None,
    };
    Some((id.to_string(), tab))
}

/// The view type of the sidebar's current tab: the `tabs` group's
/// `currentTab` child (the first when the key is absent).
fn dock_current_view(layout: &Value, dock: &str) -> Option<String> {
    let mut groups = Vec::new();
    fn find_groups<'a>(node: &'a Value, out: &mut Vec<&'a Value>) {
        if node.get("type").and_then(Value::as_str) == Some("tabs") {
            out.push(node);
        }
        if let Some(children) = node.get("children").and_then(Value::as_array) {
            for child in children {
                find_groups(child, out);
            }
        }
    }
    find_groups(layout.get(dock)?, &mut groups);
    let group = groups.first()?;
    let ix = group.get("currentTab").and_then(Value::as_u64).unwrap_or(0) as usize;
    let leaf = group.get("children")?.as_array()?.get(ix)?;
    leaf.get("state")?.get("type")?.as_str().map(str::to_string)
}

/// A pane's filter field: Obsidian's `search-input-container`, its text
/// 30 px in to leave room for the magnifier drawn over it.
fn pane_filter_input(cx: &mut Context<TextInput>) -> TextInput {
    let mut input = TextInput::new("Search...", cx);
    input.set_pad_left(px(30.));
    input
}

/// Scrolls the Outline so the marked row is in view with Obsidian's
/// margins: a row below the view stops 32 px above the bottom edge (the
/// pane's own bottom padding, which keeps it clear of the status bar), a
/// row above it goes to the top edge, and the first row takes the list
/// back to the very top. `false` until the rows have been laid out, so the
/// caller tries again next frame.
fn scroll_outline_to(scroll: &ScrollHandle, ix: usize) -> bool {
    let Some(row) = scroll.bounds_for_item(ix) else {
        return false;
    };
    let view = scroll.bounds();
    if view.size.height <= px(0.) {
        return false;
    }
    let mut offset = scroll.offset();
    let floor = view.bottom() - px(32.);
    if ix == 0 {
        offset.y = px(0.);
    } else if row.bottom() > floor {
        offset.y -= row.bottom() - floor;
    } else if row.top() < view.top() {
        offset.y += view.top() - row.top();
    } else {
        return true;
    }
    scroll.set_offset(offset);
    true
}

/// Obsidian's default dock, as a fresh vault's `workspace.json` has it:
/// a horizontal split, 300 wide, holding one tab group of the side's
/// default leaves with their default state, `currentTab` at `tab`. The
/// `file` keys name the open note and are left out when there is none.
fn default_dock_json(dock: &str, current: Option<&str>, tab: usize) -> Value {
    let leaf = |kind: &str, state: Value, icon: &str, title: &str| {
        let mut state = state;
        if current.is_none() {
            if let Some(map) = state.as_object_mut() {
                map.shift_remove("file");
            }
        }
        json!({
            "id": leaf_id(),
            "type": "leaf",
            "state": {"type": kind, "state": state, "icon": icon, "title": title},
        })
    };
    let children = if dock == "left" {
        vec![
            leaf(
                "file-explorer",
                json!({"sortOrder": "alphabetical", "autoReveal": false, "showSearch": false, "searchQuery": ""}),
                "lucide-folder-closed",
                "Files",
            ),
            leaf(
                "search",
                json!({"query": "", "matchingCase": false, "explainSearch": false, "collapseAll": false, "extraContext": false, "sortOrder": "alphabetical"}),
                "lucide-search",
                "Search",
            ),
            leaf(
                "bookmarks",
                json!({"showSearch": false, "searchQuery": ""}),
                "lucide-bookmark",
                "Bookmarks",
            ),
        ]
    } else {
        vec![
            leaf(
                "backlink",
                json!({"file": current, "collapseAll": false, "extraContext": false, "sortOrder": "alphabetical", "showSearch": false, "searchQuery": "", "backlinkCollapsed": false, "unlinkedCollapsed": true}),
                "links-coming-in",
                "Backlinks",
            ),
            leaf(
                "outgoing-link",
                json!({"file": current, "linksCollapsed": false, "unlinkedCollapsed": true}),
                "links-going-out",
                "Outgoing links",
            ),
            leaf(
                "tag",
                json!({"sortOrder": "frequency", "useHierarchy": true, "showSearch": false, "searchQuery": ""}),
                "lucide-tags",
                "Tags",
            ),
            leaf(
                "all-properties",
                json!({"sortOrder": "frequency", "showSearch": false, "searchQuery": ""}),
                "lucide-archive",
                "All properties",
            ),
            leaf(
                "outline",
                json!({"file": current, "followCursor": false, "showSearch": false, "searchQuery": ""}),
                "lucide-list",
                "Outline",
            ),
        ]
    };
    let tab = tab.min(children.len().saturating_sub(1));
    json!({
        "id": leaf_id(),
        "type": "split",
        "children": [{"id": leaf_id(), "type": "tabs", "children": children, "currentTab": tab}],
        "direction": "horizontal",
        "width": SIDEBAR_WIDTH as i64,
    })
}

/// The first leaf of view type `kind` anywhere under a dock's tree, as
/// the leaf node.
fn leaf_of_kind_mut<'a>(node: &'a mut Value, kind: &str) -> Option<&'a mut Map<String, Value>> {
    let is_leaf = node.get("type").and_then(Value::as_str) == Some("leaf")
        && node
            .get("state")
            .and_then(|s| s.get("type"))
            .and_then(Value::as_str)
            == Some(kind);
    if is_leaf {
        return node.as_object_mut();
    }
    let children = node.get_mut("children")?.as_array_mut()?;
    children.iter_mut().find_map(|c| leaf_of_kind_mut(c, kind))
}

fn leaf_of_kind<'a>(node: &'a Value, kind: &str) -> Option<&'a Map<String, Value>> {
    let is_leaf = node.get("type").and_then(Value::as_str) == Some("leaf")
        && node
            .get("state")
            .and_then(|s| s.get("type"))
            .and_then(Value::as_str)
            == Some(kind);
    if is_leaf {
        return node.as_object();
    }
    let children = node.get("children")?.as_array()?;
    children.iter().find_map(|c| leaf_of_kind(c, kind))
}

/// The `state.state` object of the first `kind` leaf under `dock`.
fn leaf_state<'a>(layout: &'a Value, dock: &str, kind: &str) -> Option<&'a Value> {
    leaf_of_kind(layout.get(dock)?, kind)?
        .get("state")?
        .get("state")
}

/// Writes `keys` into the state of the first `kind` leaf under `dock`, in
/// the place they had; a leaf without a state object gains one holding
/// only them, and a `null` value removes its key. A dock without the leaf
/// is left alone.
fn set_leaf_state(dock: &mut Value, kind: &str, keys: &[(&str, Value)]) {
    let Some(leaf) = leaf_of_kind_mut(dock, kind) else {
        return;
    };
    let Some(outer) = leaf.get_mut("state").and_then(Value::as_object_mut) else {
        return;
    };
    if !outer.get("state").is_some_and(Value::is_object) {
        outer.insert("state".into(), json!({}));
    }
    if let Some(state) = outer.get_mut("state").and_then(Value::as_object_mut) {
        for (key, value) in keys {
            if value.is_null() {
                state.remove(*key);
            } else {
                state.insert((*key).into(), value.clone());
            }
        }
    }
}

/// The `file-explorer` leaf anywhere under a dock's tree, as the leaf node.
fn explorer_leaf(node: &Value) -> Option<&Map<String, Value>> {
    leaf_of_kind(node, "file-explorer")
}

/// The Files leaf's `sortOrder` and `autoReveal` from `workspace.json`'s
/// left dock, each at Obsidian's default (`alphabetical`, off) when the key
/// is missing or unreadable; `None` when the dock has no such leaf.
fn explorer_leaf_state(layout: &Value) -> Option<(SortOrder, bool)> {
    let leaf = explorer_leaf(layout.get("left")?)?;
    let state = leaf.get("state").and_then(|s| s.get("state"));
    let sort = state
        .and_then(|s| s.get("sortOrder"))
        .and_then(Value::as_str)
        .and_then(SortOrder::parse)
        .unwrap_or_default();
    let auto_reveal = state
        .and_then(|s| s.get("autoReveal"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Some((sort, auto_reveal))
}

/// Writes the two keys into the Files leaf's state under `dock`, in the
/// place they had; a leaf without a state object gains one holding only
/// them. A dock without the leaf is left alone.
fn set_explorer_leaf_state(dock: &mut Value, sort: SortOrder, auto_reveal: bool) {
    set_leaf_state(
        dock,
        "file-explorer",
        &[
            ("sortOrder", json!(sort.as_str())),
            ("autoReveal", json!(auto_reveal)),
        ],
    );
}

/// A fresh leaf id in Obsidian's shape: sixteen hex digits.
fn leaf_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut x = nanos
        ^ (COUNTER
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15));
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    format!("{x:016x}")
}

/// The `workspace.json` leaf for a tab: the one it was restored from with
/// its state brought up to date, or a new one.
fn leaf_json(tab: &Tab) -> Value {
    let (mode, source) = match tab.mode {
        ViewMode::Reading => ("preview", false),
        ViewMode::Source => ("source", true),
        ViewMode::LivePreview | ViewMode::Graph => ("source", false),
    };
    let title = tab.title();
    let mut leaf = tab
        .leaf
        .clone()
        .filter(|l| {
            let file = l
                .get("state")
                .and_then(|s| s.get("state"))
                .and_then(|s| s.get("file"))
                .and_then(Value::as_str);
            let view = l
                .get("state")
                .and_then(|s| s.get("type"))
                .and_then(Value::as_str);
            match (&tab.path, tab.mode) {
                (Some(p), _) => file == Some(p.as_str()),
                (None, ViewMode::Graph) => view == Some("graph"),
                (None, _) => view == Some("empty"),
            }
        })
        .unwrap_or_else(|| match (&tab.path, tab.mode) {
            (Some(p), _) => json!({
                "id": leaf_id(),
                "type": "leaf",
                "state": {
                    "type": leaf_view_type(p),
                    "state": {"file": p},
                    "icon": leaf_icon(p),
                    "title": title,
                },
            }),
            (None, ViewMode::Graph) => json!({
                "id": leaf_id(),
                "type": "leaf",
                "state": {"type": "graph", "state": {}, "icon": "lucide-git-fork", "title": title},
            }),
            (None, _) => json!({
                "id": leaf_id(),
                "type": "leaf",
                "state": {"type": "empty", "state": {}, "icon": "lucide-file", "title": title},
            }),
        });
    if let Some(path) = &tab.path {
        if leaf_view_type(path) == "markdown" {
            if let Some(inner) = leaf
                .get_mut("state")
                .and_then(|s| s.get_mut("state"))
                .and_then(Value::as_object_mut)
            {
                inner.insert("mode".into(), json!(mode));
                inner.insert("source".into(), json!(source));
            }
        }
        // A canvas leaf records where the view looked, when it is known; a
        // tab restored with a `viewState` and never moved keeps the one it
        // was read with.
        if leaf_view_type(path) == "canvas" {
            if let Some(view_state) = &tab.view_state {
                if let Some(inner) = leaf
                    .get_mut("state")
                    .and_then(|s| s.get_mut("state"))
                    .and_then(Value::as_object_mut)
                {
                    inner.insert("viewState".into(), view_state.clone());
                }
            }
        }
        if let Some(state) = leaf.get_mut("state").and_then(Value::as_object_mut) {
            state.insert("title".into(), json!(title));
        }
    }
    leaf
}

/// The Lucide icon Obsidian records on a file's leaf: the dashboard for a
/// canvas, the list layout for a base, the text file for a PDF (observed:
/// `tools/obsidian-probe/results/2026-10-26-pdf.json`), the plain file
/// otherwise.
///
/// OBSERVE: what it writes for images and the rest, and the base's
/// own leaf icon -- the Bases probe of 2026-10-25 never read a leaf, so
/// `lucide-layout-list` here is the ribbon button's observed icon (see
/// [[Specification#Observed Bases write behavior, Obsidian 1.14.5
/// (installer build 1.13.7)]]) carried over the way the canvas's is.
fn leaf_icon(path: &str) -> &'static str {
    match FileKind::from_path(path) {
        FileKind::Canvas => "lucide-layout-dashboard",
        FileKind::Base => "lucide-layout-list",
        FileKind::Pdf => "lucide-file-text",
        _ => "lucide-file",
    }
}

/// One foot of the active tab: the region just outside its bottom corner
/// that lies between the tab's edge, the strip's bottom and a quarter-circle
/// centred on the strip side, painted in the tab's colour. Only that region
/// is painted, so a neighbouring tab's hover wash shows through the curve.
/// What shows is the tab's edge curving down into the view header; the
/// outline hairline stops where the curve begins.
fn tab_foot(left: bool, theme: &Theme) -> impl IntoElement {
    let r = px(TAB_CURVE);
    let color = theme.bg;
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let o = bounds.origin;
            let mut path = PathBuilder::fill();
            if left {
                // The arc is centred on the top-left corner: from the
                // bottom-left point round to the top-right one.
                path.move_to(point(o.x + r, o.y));
                path.line_to(point(o.x + r, o.y + r));
                path.line_to(point(o.x, o.y + r));
                path.arc_to(point(r, r), px(0.), false, false, point(o.x + r, o.y));
            } else {
                // Mirrored: the arc is centred on the top-right corner.
                path.move_to(point(o.x, o.y));
                path.line_to(point(o.x, o.y + r));
                path.line_to(point(o.x + r, o.y + r));
                path.arc_to(point(r, r), px(0.), false, true, point(o.x, o.y));
            }
            path.close();
            if let Ok(built) = path.build() {
                window.paint_path(built, color);
            }
        },
    )
    .absolute()
    .bottom_0()
    .w(r)
    .h(r)
    .map(|d| {
        if left {
            d.left(px(-TAB_CURVE))
        } else {
            d.right(px(-TAB_CURVE))
        }
    })
}

/// The tab strip's name for a file: a note's name without `.md`, and a
/// canvas without `.canvas`, a base without `.base` or a PDF without
/// `.pdf` too, which is how Obsidian titles its tabs -- no view of its own
/// ever shows an extension (the PDF's is observed: `doc.pdf` is the tab
/// "doc" and the leaf's `title` "doc").
fn tab_display_name(file_name: &str) -> &str {
    match file_name.rsplit_once('.') {
        Some((stem, ext))
            if (ext.eq_ignore_ascii_case("canvas")
                || ext.eq_ignore_ascii_case("base")
                || ext.eq_ignore_ascii_case("pdf"))
                && !stem.is_empty() =>
        {
            stem
        }
        _ => display_name(file_name),
    }
}

/// What a leaf showing `path` in `mode` is called, which is the one string
/// the tab strip, the view header and the window title all show: the file's
/// display name, Obsidian's "New tab" for a leaf of type `empty` and
/// "Graph view" for the graph (see [[Specification#The empty tab]] -- both
/// the empty leaf's display text and its view-header title read "New tab",
/// not "No file is open").
fn leaf_title(path: Option<&str>, mode: ViewMode) -> SharedString {
    match (path, mode) {
        (Some(p), _) => tab_display_name(p.rsplit('/').next().unwrap_or(p))
            .to_string()
            .into(),
        (None, ViewMode::Graph) => "Graph view".into(),
        (None, _) => "New tab".into(),
    }
}

/// The leaf view type Obsidian gives a file of this kind.
fn leaf_view_type(path: &str) -> &'static str {
    match FileKind::from_path(path) {
        FileKind::Markdown => "markdown",
        FileKind::Image => "image",
        FileKind::Pdf => "pdf",
        FileKind::Canvas => "canvas",
        // OBSERVE: unread, like the base's leaf icon; `bases` is the id of
        // the core plugin that registers the view.
        FileKind::Base => "bases",
        FileKind::Audio => "audio",
        FileKind::Video => "video",
        _ => "markdown",
    }
}

/// Where typed or composed text lands in the title field: the range the
/// platform named, else the composition it has open, else the selection --
/// so a keystroke with text selected replaces it.
fn title_input_range(edit: &TitleEdit, range_utf16: Option<Range<usize>>) -> Range<usize> {
    let text = edit.field.text();
    range_utf16
        .map(|r| title_field::utf16_to_byte(text, r.start)..title_field::utf16_to_byte(text, r.end))
        .or_else(|| edit.marked.clone())
        .unwrap_or_else(|| edit.field.selection())
}

/// Typed, composed and pasted text while the view header's title is being
/// renamed in place. Text comes in through GPUI's input handler so IME
/// composition and dead keys work, as they do in the crate's other field.
///
/// The handler is registered by the title's own element against the title's
/// own focus handle (see [`Workspace::render_title`]), so it only ever sees
/// text meant for that field; with no field open every method answers as an
/// empty one would.
impl EntityInputHandler for Workspace {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let edit = self.title_edit.as_ref()?;
        let text = edit.field.text();
        let start = title_field::utf16_to_byte(text, range_utf16.start);
        let end = title_field::utf16_to_byte(text, range_utf16.end);
        adjusted_range.replace(
            title_field::byte_to_utf16(text, start)..title_field::byte_to_utf16(text, end),
        );
        Some(text[start..end].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let edit = self.title_edit.as_ref()?;
        let text = edit.field.text();
        let selection = edit.field.selection();
        Some(UTF16Selection {
            range: title_field::byte_to_utf16(text, selection.start)
                ..title_field::byte_to_utf16(text, selection.end),
            reversed: edit.field.reversed(),
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        let edit = self.title_edit.as_ref()?;
        let marked = edit.marked.clone()?;
        let text = edit.field.text();
        Some(
            title_field::byte_to_utf16(text, marked.start)
                ..title_field::byte_to_utf16(text, marked.end),
        )
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        if let Some(edit) = self.title_edit.as_mut() {
            edit.marked = None;
        }
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(edit) = self.title_edit.as_mut() else {
            return;
        };
        let range = title_input_range(edit, range_utf16);
        edit.field.replace(range, new_text);
        edit.marked = None;
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(edit) = self.title_edit.as_mut() else {
            return;
        };
        let range = title_input_range(edit, range_utf16);
        let landed = edit.field.replace(range.clone(), new_text);
        let marked = range.start..range.start + landed;
        if let Some(selected) = new_selected_range {
            let at = marked.start + title_field::utf16_to_byte(new_text, selected.end);
            edit.field.move_to(at, false);
        }
        edit.marked = Some(marked);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let edit = self.title_edit.as_ref()?;
        let layout = self.title_layout.get(&edit.group)?;
        let text = edit.field.text();
        let start = title_field::utf16_to_byte(text, range_utf16.start);
        let end = title_field::utf16_to_byte(text, range_utf16.end);
        let x0 = element_bounds.origin.x - layout.scroll_x;
        Some(Bounds::from_corners(
            point(x0 + layout.line.x_for_index(start), element_bounds.origin.y),
            point(
                x0 + layout.line.x_for_index(end),
                element_bounds.origin.y + layout.line_height,
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        _point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.corners = WindowCorners::of(window);
        let corners = self.corners;
        if let Some(kind) = self.pending_canvas_pick.take() {
            self.open_canvas_file_chooser(kind, window, cx);
        }
        // A view's focus handle only joins the dispatch tree when the frame
        // that paints it is prepainted, so this — the start of that frame —
        // is the first moment a focus asked for elsewhere can land. A modal
        // owns the keyboard while it is up, and closing one focuses the main
        // area anyway, so the request is dropped rather than queued.
        if std::mem::take(&mut self.refocus_main)
            && self.modal == Modal::None
            && self.keyboard_is_parked(window, cx)
        {
            self.focus_main(window, cx);
        }
        // A title edit belongs to the file whose header it opened on: if
        // that leaf moved on -- another tab, another file, the rename
        // itself -- the field goes with it rather than committing a name to
        // whatever is on screen now.
        if let Some(edit) = &self.title_edit {
            if self.group_current(edit.group) != Some(edit.path.as_str()) {
                self.title_edit = None;
                if self.title_focus.is_focused(window) {
                    self.focus_main(window, cx);
                }
            }
        }
        // A drag that ended anywhere but on a drop zone just stops: gpui
        // drops it on the mouse up and redraws, and this is where the zones
        // learn to go away.
        if self.dragging_tab && !cx.has_active_drag() {
            self.dragging_tab = false;
            self.tab_drop = None;
            self.drop_bar = None;
            self.tab_bounds.clear();
        }
        // A sliding drop bar needs a frame per step until it lands.
        if self.drop_bar.is_some_and(|b| !b.settled(Instant::now())) {
            window.request_animation_frame();
        }
        let theme = cx.global::<Theme>().clone();
        // The active leaf's name, which is the window title's first part:
        // "New tab" with nothing open, as Obsidian's reads
        // (`New tab - <vault> - Obsidian 1.14.5`).
        let title = leaf_title(self.current.as_deref(), self.mode);

        let main_body: gpui::AnyElement = if let Some(loading) = &self.loading {
            div()
                .p_4()
                .text_color(theme.text_muted)
                .child(loading.clone())
                .into_any_element()
        } else {
            let tree = self.layout_tree.clone();
            self.render_layout(&tree, &mut Vec::new(), &theme, cx)
        };

        let wanted_title: SharedString = format!("{title} - {} - Silica", self.vault_name).into();
        if self.window_title != wanted_title {
            window.set_window_title(&wanted_title);
            self.window_title = wanted_title;
        }

        let main = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .child(main_body);

        let show_ribbon = self.show_ribbon();
        let frameless = frameless(window);
        self.frameless = frameless;
        self.fullscreen = window.is_fullscreen();
        let mac = self.mac_frame();
        let ribbon = show_ribbon.then(|| self.render_ribbon(&theme, cx));
        // As on the right: a pane whose plugin is off has no tab, so it
        // cannot be the one showing.
        for (tab, id) in [
            (LeftTab::Files, "file-explorer"),
            (LeftTab::Search, "global-search"),
            (LeftTab::Bookmarks, "bookmarks"),
        ] {
            if self.left_tab == tab && !self.left_pane_enabled(id) {
                self.left_tab = [
                    (LeftTab::Files, "file-explorer"),
                    (LeftTab::Search, "global-search"),
                    (LeftTab::Bookmarks, "bookmarks"),
                ]
                .into_iter()
                .find(|(_, id)| self.left_pane_enabled(id))
                .map(|(t, _)| t)
                .unwrap_or(tab);
            }
        }
        let left_tab = self.left_tab;
        let left_body: gpui::AnyElement = match left_tab {
            LeftTab::Files => self.explorer.clone().into_any_element(),
            LeftTab::Search => self.search.clone().into_any_element(),
            LeftTab::Bookmarks => self.bookmarks.clone().into_any_element(),
        };
        let bookmarks_on = self.state.as_ref().is_some_and(|s| s.bookmarks_enabled());
        let left = div()
            .relative()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .w(px(self.left_width as f32))
            .h_full()
            .bg(theme.bg_secondary)
            .when(!show_ribbon, |d| corners.left().apply(d))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .h(px(HEADER_HEIGHT))
                    // Past the traffic lights on macOS, where this header
                    // is the window's top-left corner.
                    .pl(px(8. + self.frame_left_space()))
                    .pr(px(8.))
                    .gap(px(3.))
                    .border_b_1()
                    .border_color(theme.border)
                    .children(self.left_pane_enabled("file-explorer").then(|| {
                        self.dock_tab(
                            "Files",
                            "folder-closed",
                            left_tab == LeftTab::Files,
                            &theme,
                            |this, window, cx| {
                                this.leave_search_field(window, cx);
                                this.left_tab = LeftTab::Files;
                                cx.notify();
                            },
                            cx,
                        )
                    }))
                    .children(self.left_pane_enabled("global-search").then(|| {
                        self.dock_tab(
                            "Search",
                            "search",
                            left_tab == LeftTab::Search,
                            &theme,
                            |this, window, cx| {
                                let q = this.search.read(cx).query(cx);
                                this.show_search(&q, Some(window), cx);
                            },
                            cx,
                        )
                    }))
                    .when(bookmarks_on, |d| {
                        d.child(self.dock_tab(
                            "Bookmarks",
                            "bookmark",
                            left_tab == LeftTab::Bookmarks,
                            &theme,
                            |this, window, cx| {
                                this.leave_search_field(window, cx);
                                this.left_tab = LeftTab::Bookmarks;
                                cx.notify();
                            },
                            cx,
                        ))
                    })
                    .child(window_handle(
                        div().id("left-dock-space").flex_1().h_full(),
                        frameless,
                    ))
                    // With the ribbon hidden, and on macOS whenever this
                    // sidebar is open, Obsidian appends the toggle to this
                    // row: it sits at the right end, against the dock's edge.
                    .when(!show_ribbon || mac, |d| {
                        d.child(self.sidebar_toggle(true, &theme, cx))
                    }),
            )
            .child(div().flex_1().overflow_hidden().child(left_body))
            .child(self.render_vault_profile(&theme, window, cx))
            .child(self.dock_handle(true, &theme));

        let status_items = self.status_bar_items(cx);
        // Obsidian's status bar is a tab in the window's bottom right
        // corner rather than a bar across it: the sidebar colour, a border
        // on its open sides, an 8 px top left corner, the smaller UI size.
        let status_bar = div()
            .absolute()
            .bottom_0()
            .right_0()
            .max_w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.))
            .pl(px(4.))
            .pr(px(8.))
            .py(px(4.))
            .rounded_tl(px(8.))
            .when(corners.br, |d| d.rounded_br(WINDOW_RADIUS))
            .bg(theme.bg_secondary)
            .border_t_1()
            .border_l_1()
            .border_color(theme.border)
            .text_size(px(FONT_UI_SMALLER))
            .text_color(theme.text_muted)
            .child(
                div()
                    .px(px(4.))
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .overflow_hidden()
                    .child(self.status.clone()),
            )
            .children(status_items.into_iter().map(|text| {
                div()
                    .px(px(4.))
                    .whitespace_nowrap()
                    .child(SharedString::from(text))
            }));

        // Every overlay occludes: a modal in Obsidian owns the pointer as
        // well as the keyboard, and without this the editor's raw window
        // listeners take the mouse-down that landed on the modal and focus
        // the note behind it, leaving the modal visible but keyboard-dead.
        // Occluding costs nothing a modal wants — a click outside still
        // reaches each list modal's `on_mouse_down_out`, which is bounds
        // based rather than hitbox based, and Settings closes on its own
        // backdrop click.
        let overlay: Option<gpui::AnyElement> = match self.modal {
            Modal::None => None,
            Modal::Switcher
            | Modal::Palette
            | Modal::Prompt
            | Modal::Chooser
            | Modal::CopyResults => {
                let child: gpui::AnyElement = match self.modal {
                    Modal::Switcher => self.switcher.clone().into_any_element(),
                    Modal::Palette => self.palette.clone().into_any_element(),
                    Modal::Chooser => self.chooser.clone().into_any_element(),
                    Modal::CopyResults => self.copy_results.clone().into_any_element(),
                    _ => self.prompt.clone().into_any_element(),
                };
                Some(
                    div()
                        .occlude()
                        .absolute()
                        .inset_0()
                        .flex()
                        .flex_row()
                        .justify_center()
                        .items_start()
                        .pt(px(90.))
                        .child(child)
                        .into_any_element(),
                )
            }
            Modal::Settings => Some(
                div()
                    .id("settings-backdrop")
                    .occlude()
                    .absolute()
                    .inset_0()
                    .bg(rgba(0x00000080))
                    .map(|d| corners.apply(d))
                    .flex()
                    .flex_row()
                    .justify_center()
                    .items_center()
                    .py(px(40.))
                    .px(px(40.))
                    .on_click(cx.listener(|this, _, window, cx| this.close_modal(window, cx)))
                    .child(
                        div()
                            .id("settings-frame")
                            .h_full()
                            // Obsidian's modal is `min(90vw, 1100px)`. The
                            // frame has to carry the width, because a frame
                            // that only clips leaves the panel laying out at
                            // its full width with the control column past
                            // the clip, where nothing can be clicked.
                            .w(relative(0.9))
                            .max_w(px(1100.))
                            .on_click(|_, _, cx| cx.stop_propagation())
                            .child(self.settings.clone()),
                    )
                    .into_any_element(),
            ),
        };
        // Obsidian reports an outcome with a Notice in the corner rather
        // than in the status bar, and the probe notes put it bottom right.
        let notices = (!self.notices.live.is_empty()).then(|| {
            div()
                .absolute()
                .bottom(px(30.))
                .right(px(16.))
                .flex()
                .flex_col()
                .items_end()
                .gap_1()
                .children(self.notices.live.iter().map(|notice| {
                    div()
                        .max_w(px(360.))
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .bg(theme.bg_secondary)
                        .border_1()
                        .border_color(theme.border)
                        .text_size(px(12.))
                        .text_color(theme.text)
                        .child(notice.text.clone())
                }))
        });

        let left_visible = self.left_visible;
        let right_visible = self.right_visible;
        // A modal owns the keyboard while it is up, the way an Obsidian
        // modal's own keymap scope keeps app hotkeys from firing behind it.
        // gpui's key contexts nest rather than replace — the overlay is a
        // child of this div — so the workspace stands its handlers down
        // instead: without them a hotkey can no longer focus a pane behind
        // the modal, or swap the modal out from under its own Escape.
        let hotkeys_live = self.modal == Modal::None;
        // macOS has the desktop's traffic lights instead.
        let controls = (frameless && !mac).then(|| self.render_window_controls(&theme, window));

        let content = div()
            .key_context("Workspace")
            .track_focus(&self.focus_handle)
            .when(hotkeys_live, |d| {
                d.on_action(cx.listener(Self::toggle_mode))
                    .on_action(cx.listener(Self::go_back))
                    .on_action(cx.listener(Self::go_forward))
                    .on_action(cx.listener(Self::open_search_action))
                    .on_action(cx.listener(Self::open_graph_action))
                    .on_action(cx.listener(Self::run_command_action))
            })
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.bg)
            // This box paints the whole content square, over the frame's
            // rounding and under its children's; a corner left square here
            // is a notch of `theme.bg` outside every rounded child.
            .map(|d| corners.apply(d))
            .font_family(theme.ui_font.clone())
            .text_size(px(crate::theme::FONT_UI_MEDIUM))
            .text_color(theme.text)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .overflow_hidden()
                    .on_drag_move(cx.listener(|this, e: &DragMoveEvent<DockDrag>, _, cx| {
                        let left = e.drag(cx).left;
                        this.drag_dock(left, e.bounds, e.event.position, cx);
                    }))
                    .children(ribbon)
                    .when(left_visible, |d| d.child(left))
                    .child(main)
                    .when(right_visible, |d| {
                        d.child(self.render_right_dock(&theme, cx))
                    }),
            )
            .child(status_bar)
            .children(controls)
            .children(overlay)
            .children(notices);
        client_frame(content.into_any_element(), &theme, window)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    /// A symlink to a folder or a file on either platform. Windows refuses
    /// to make one without Developer Mode or elevation, so a test that
    /// needs one skips that part when creation is denied there.
    fn symlink(target: &Path, link: &Path) -> Option<()> {
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(target, link);
        #[cfg(windows)]
        let made = if target.is_dir() {
            std::os::windows::fs::symlink_dir(target, link)
        } else {
            std::os::windows::fs::symlink_file(target, link)
        };
        match made {
            Ok(()) => Some(()),
            Err(e) if cfg!(windows) => {
                eprintln!("symlinks are not permitted here, skipping: {e}");
                None
            }
            Err(e) => panic!("symlink {}: {e}", link.display()),
        }
    }

    fn vault() -> (tempfile::TempDir, VaultState) {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        fs::create_dir_all(r.join("Sub")).unwrap();
        fs::create_dir_all(r.join(".obsidian")).unwrap();
        fs::write(r.join(".obsidian/app.json"), r#"{"trashOption":"local"}"#).unwrap();
        fs::write(r.join("A.md"), "# A\n\nSee [[B]] and [S](Sub/S.md).\n").unwrap();
        fs::write(
            r.join("B.md"),
            "\u{feff}---\r\nrelated: \"[[A]]\"\r\n---\r\n[[A]] and [[A#A|the a]] and `[[A]]`\r\n",
        )
        .unwrap();
        fs::write(r.join("Sub/S.md"), "[A](../A.md) [[B]]\n").unwrap();
        let state = VaultState::load(&r.to_path_buf()).unwrap();
        (dir, state)
    }

    /// A link written in a property is an outgoing link like any other,
    /// listed first the way the file writes it; a body link is labelled as
    /// it reads in the note and carries the link it was written with.
    #[test]
    fn outgoing_links_list_a_property_link_too() {
        let (_dir, state) = vault();
        let cache = state.cache();
        let section = backlinks::outgoing_links(&cache, "B.md");
        type Row<'a> = (Option<&'a str>, &'a str, Option<&'a str>, Option<&'a str>);
        let rows: Vec<Row<'_>> = section
            .groups
            .iter()
            .map(|g| {
                (
                    g.path.as_deref(),
                    g.label.as_str(),
                    g.link.as_deref(),
                    g.subtext.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            rows,
            vec![
                (Some("A.md"), "A", Some("A"), None),
                (Some("A.md"), "A", Some("A#A"), Some("A")),
            ],
            "the frontmatter link and the body's [[A]] are one row; the heading link its own"
        );
        assert_eq!(section.count, 2);
    }

    /// The Linked mentions match for a property link is named by its
    /// property; a body link shows its line with the link itself marked,
    /// and the matches are grouped by the note they are in.
    #[test]
    fn a_backlink_match_names_the_property_it_was_written_in() {
        let (_dir, state) = vault();
        let cache = state.cache();
        let texts = state.texts();
        let section = backlinks::linked_mentions(&cache, &texts, "A.md", false);
        let shown: Vec<(&str, &str, Vec<&str>)> = section
            .groups
            .iter()
            .flat_map(|g| {
                g.matches.iter().map(move |m| {
                    (
                        g.path.as_deref().unwrap(),
                        m.text.as_str(),
                        m.ranges.iter().map(|r| &m.text[r.clone()]).collect(),
                    )
                })
            })
            .collect();
        assert_eq!(
            shown,
            vec![
                ("B.md", "related", vec![]),
                ("B.md", "[[A]] and [[A#A|the a]] and `[[A]]`", vec!["[[A]]"]),
                (
                    "B.md",
                    "[[A]] and [[A#A|the a]] and `[[A]]`",
                    vec!["[[A#A|the a]]"]
                ),
                ("Sub/S.md", "[A](../A.md) [[B]]", vec!["[A](../A.md)"]),
            ]
        );
        assert_eq!(section.groups.len(), 2);
        assert_eq!(section.count, 4);
    }

    /// A side Obsidian never laid out gets its default dock: one split of
    /// one tab group holding the side's leaves in Obsidian's order with
    /// their default state, `currentTab` at the open tab, and no `file`
    /// key when nothing is open.
    #[test]
    fn a_missing_dock_is_written_in_obsidians_default_shape() {
        let right = default_dock_json("right", Some("A.md"), 4);
        assert_eq!(right["type"], "split");
        assert_eq!(right["direction"], "horizontal");
        assert_eq!(right["width"], 300);
        let group = &right["children"][0];
        assert_eq!(group["type"], "tabs");
        assert_eq!(group["currentTab"], 4);
        let kinds: Vec<&str> = group["children"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["state"]["type"].as_str().unwrap())
            .collect();
        assert_eq!(
            kinds,
            [
                "backlink",
                "outgoing-link",
                "tag",
                "all-properties",
                "outline"
            ]
        );
        let outline = &group["children"][4]["state"];
        assert_eq!(outline["icon"], "lucide-list");
        assert_eq!(outline["title"], "Outline");
        assert_eq!(
            outline["state"],
            json!({"file": "A.md", "followCursor": false, "showSearch": false, "searchQuery": ""})
        );
        assert_eq!(
            group["children"][2]["state"]["state"],
            json!({"sortOrder": "frequency", "useHierarchy": true, "showSearch": false, "searchQuery": ""})
        );
        assert_eq!(group["children"][0]["id"].as_str().unwrap().len(), 16);

        let bare = default_dock_json("right", None, 9);
        let group = &bare["children"][0];
        assert_eq!(group["currentTab"], 4, "clamped to the last tab");
        assert!(group["children"][0]["state"]["state"].get("file").is_none());
        assert!(group["children"][4]["state"]["state"].get("file").is_none());

        let left = default_dock_json("left", None, 1);
        let kinds: Vec<&str> = left["children"][0]["children"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["state"]["type"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, ["file-explorer", "search", "bookmarks"]);
    }

    /// The Backlinks and Outgoing links leaves' state round trips: read
    /// at Obsidian's defaults when missing, written in place.
    #[test]
    fn pane_leaf_state_is_read_and_written_in_place() {
        let mut layout = json!({
            "right": {"type": "split", "children": [{
                "type": "tabs",
                "children": [
                    {"type": "leaf", "state": {"type": "backlink", "state": {
                        "file": "A.md", "collapseAll": true, "sortOrder": "byModifiedTime",
                        "unlinkedCollapsed": true
                    }}},
                    {"type": "leaf", "state": {"type": "outgoing-link"}},
                ],
            }]},
        });
        let state = leaf_state(&layout, "right", "backlink").unwrap();
        assert_eq!(state["collapseAll"], true);
        assert_eq!(state.get("extraContext"), None);
        assert_eq!(leaf_state(&layout, "right", "outgoing-link"), None);
        assert_eq!(leaf_state(&layout, "left", "backlink"), None);

        set_leaf_state(
            &mut layout["right"],
            "backlink",
            &[("collapseAll", json!(false)), ("showSearch", json!(true))],
        );
        set_leaf_state(
            &mut layout["right"],
            "outgoing-link",
            &[("linksCollapsed", json!(true))],
        );
        let leaves = &layout["right"]["children"][0]["children"];
        let keys: Vec<&String> = leaves[0]["state"]["state"]
            .as_object()
            .unwrap()
            .keys()
            .collect();
        assert_eq!(
            keys,
            [
                "file",
                "collapseAll",
                "sortOrder",
                "unlinkedCollapsed",
                "showSearch"
            ]
        );
        assert_eq!(leaves[0]["state"]["state"]["collapseAll"], false);
        assert_eq!(
            leaves[1]["state"]["state"],
            json!({"linksCollapsed": true}),
            "a leaf without a state gains one"
        );
    }

    fn history(entries: &[&str]) -> Vec<HistoryEntry> {
        entries
            .iter()
            .map(|e| HistoryEntry::new(e.to_string(), ViewMode::LivePreview))
            .collect()
    }

    /// `can_run` checks `RUNNABLE` first, so an id only in `EDITOR_ONLY`
    /// would be filtered twice over and never offered at all.
    #[test]
    fn every_editor_only_id_is_runnable() {
        for id in EDITOR_ONLY {
            assert!(
                RUNNABLE.contains(id),
                "{id} is editor-only but not runnable"
            );
        }
        // The table commands have their own rule in `can_run` and must not
        // be in the editor-only list as well, or the narrower check never
        // runs.
        for id in EDITOR_ONLY {
            assert!(
                TableCommand::from_id(id).is_none(),
                "{id} is both a table command and editor-only"
            );
        }
    }

    /// Neither list may name an id the registry does not have: a typo here
    /// is a command that silently never appears.
    #[test]
    fn both_lists_name_real_commands() {
        let registry = CommandRegistry::with_known_ids();
        for id in RUNNABLE.iter().chain(EDITOR_ONLY) {
            assert!(registry.get(id).is_some(), "{id} is not a known command");
        }
    }

    /// The converse of [`every_editor_only_id_is_runnable`], and the one
    /// that was missing: an id `run_command` answers but `RUNNABLE` does not
    /// name is a command that works and nothing can reach. `can_run` tests
    /// `RUNNABLE` first, so the palette leaves the id out, the socket
    /// reports it as not runnable, `bind_keys` never binds its hotkey and
    /// `run_command` does not even count it as recent. "Open help" sat that
    /// way with F1 in `silica-commands` and no key bound to it.
    ///
    /// The ids are read off the match itself rather than listed again here,
    /// because a list kept by hand is what failed the first time.
    #[test]
    fn every_handled_id_is_runnable() {
        const SOURCE: &str = include_str!("workspace.rs");
        let body = SOURCE
            .split_once("pub fn run_command(")
            .expect("run_command is declared in this file")
            .1;
        let mut handled: Vec<String> = Vec::new();
        // An arm's pattern is the lines at the match's own indentation that
        // start with an id or carry on an `|` chain, up to the `=>`: a long
        // chain is wrapped over several lines, and a guard (`if ...`) sits
        // between the last id and the arrow. An arm's body is indented
        // past this, so a nested match and a string literal in a call are
        // both out of reach.
        let mut pattern = String::new();
        for line in body.lines() {
            if line.starts_with("            _ =>") {
                break;
            }
            if !(line.starts_with("            \"") || line.starts_with("            | \"")) {
                continue;
            }
            pattern.push_str(line);
            let Some((ids, _)) = pattern.split_once("=>") else {
                continue;
            };
            // Every id is quoted, so the odd pieces of a split on the quote
            // are the ids and the even ones are the `|`s and the guard.
            handled.extend(ids.split('"').skip(1).step_by(2).map(|id| id.to_string()));
            pattern.clear();
        }
        // A floor, so that a change to the match's shape shows up as a
        // failure here rather than as a test that quietly checks nothing.
        assert!(
            handled.len() >= 95,
            "only {} arms found; the reader above has stopped matching \
             run_command's match",
            handled.len()
        );
        for id in &handled {
            assert!(
                RUNNABLE.contains(&id.as_str()),
                "{id} is handled by run_command but not in RUNNABLE, so \
                 nothing can reach it"
            );
        }
    }

    /// A deleted note leaves the history, so back and forward cannot land
    /// on a file the vault no longer has. Entries before the cursor take it
    /// back with them; the notes it can still reach do not change.
    fn leaf(id: &str, file: &str, mode: &str, source: bool) -> Value {
        json!({
            "id": id,
            "type": "leaf",
            "state": {
                "type": "markdown",
                "state": {"file": file, "mode": mode, "source": source},
                "icon": "lucide-file",
                "title": file.rsplit('/').next().unwrap().trim_end_matches(".md"),
                "pinned": true,
            },
        })
    }

    #[test]
    fn a_restored_leaf_is_written_back_as_it_was_read() {
        let read = leaf("a1072f377b3d1c69", "Sub/Note.md", "source", false);
        let tab = Tab {
            path: Some("Sub/Note.md".into()),
            mode: ViewMode::LivePreview,
            history: vec![HistoryEntry::new(
                "Sub/Note.md".into(),
                ViewMode::LivePreview,
            )],
            history_ix: 1,
            leaf: Some(read.clone()),
            view_state: None,
        };
        assert_eq!(leaf_json(&tab), read);
    }

    #[test]
    fn a_mode_change_rewrites_only_the_mode_keys() {
        let read = leaf("a1072f377b3d1c69", "Note.md", "source", false);
        let tab = Tab {
            path: Some("Note.md".into()),
            mode: ViewMode::Reading,
            history: Vec::new(),
            history_ix: 0,
            leaf: Some(read.clone()),
            view_state: None,
        };
        let written = leaf_json(&tab);
        assert_eq!(written["id"], read["id"]);
        assert_eq!(written["state"]["pinned"], json!(true));
        assert_eq!(written["state"]["state"]["mode"], json!("preview"));
        assert_eq!(written["state"]["state"]["source"], json!(false));
        let tab = Tab {
            mode: ViewMode::Source,
            ..tab
        };
        let written = leaf_json(&tab);
        assert_eq!(written["state"]["state"]["mode"], json!("source"));
        assert_eq!(written["state"]["state"]["source"], json!(true));
    }

    /// A canvas tab is written as Obsidian's canvas leaf: its view type and
    /// icon, the title without the extension, and the `viewState` the view
    /// reported; a leaf restored with one and never moved keeps it.
    #[test]
    fn a_canvas_tab_is_written_as_a_canvas_leaf() {
        let tab = Tab {
            path: Some("Plans/Board.canvas".into()),
            mode: ViewMode::LivePreview,
            history: Vec::new(),
            history_ix: 0,
            leaf: None,
            view_state: Some(json!({"x": 100, "y": -20, "zoom": -1})),
        };
        let written = leaf_json(&tab);
        assert_eq!(written["state"]["type"], json!("canvas"));
        assert_eq!(written["state"]["icon"], json!("lucide-layout-dashboard"));
        assert_eq!(written["state"]["title"], json!("Board"));
        assert_eq!(
            written["state"]["state"],
            json!({"file": "Plans/Board.canvas", "viewState": {"x": 100, "y": -20, "zoom": -1}})
        );
        assert!(written["state"]["state"].get("mode").is_none());
        let known = |_: &str| true;
        let (_, restored) = tab_from_leaf(&written, ViewMode::Reading, &known).unwrap();
        assert_eq!(restored.view_state, tab.view_state);
        assert_eq!(leaf_json(&restored), written);
    }

    /// A base tab is written as a Bases leaf, not a markdown one: the
    /// extension never reaches the title, and neither of the markdown
    /// leaf's `mode`/`source` keys is written. A base has no `viewState`
    /// of its own -- that is the canvas's viewport, and the Bases probe
    /// read no leaf at all, so nothing is invented here.
    #[test]
    fn a_base_tab_is_written_as_a_bases_leaf() {
        let tab = Tab {
            path: Some("Shelf/Books.base".into()),
            mode: ViewMode::LivePreview,
            history: Vec::new(),
            history_ix: 0,
            leaf: None,
            view_state: None,
        };
        let written = leaf_json(&tab);
        assert_eq!(written["state"]["type"], json!("bases"));
        assert_eq!(written["state"]["icon"], json!("lucide-layout-list"));
        assert_eq!(written["state"]["title"], json!("Books"));
        assert_eq!(
            written["state"]["state"],
            json!({"file": "Shelf/Books.base"})
        );
        assert!(written["state"]["state"].get("mode").is_none());
        assert!(written["state"]["state"].get("viewState").is_none());
        assert_eq!(tab.title(), "Books", "the strip shows no extension either");
        assert_eq!(
            tab_display_name("Books.BASE"),
            "Books",
            "the extension is matched without case"
        );
        assert_eq!(
            tab_display_name(".base"),
            ".base",
            "a dotfile is its own name"
        );
        let known = |_: &str| true;
        let (_, restored) = tab_from_leaf(&written, ViewMode::Reading, &known).unwrap();
        assert_eq!(leaf_json(&restored), written);
    }

    /// A PDF tab is written as Obsidian's `pdf` leaf, which is the file
    /// alone: observed, `state.state` is `{"file": ..}` before and after
    /// scrolling, zooming and a quit, the icon is `lucide-file-text` and
    /// the title drops the extension. Where the view is in the file never
    /// reaches the leaf, and a leaf read with extra keys is written back as
    /// read.
    #[test]
    fn a_pdf_tab_is_written_as_a_file_only_pdf_leaf() {
        let tab = Tab {
            path: Some("Papers/Field.pdf".into()),
            mode: ViewMode::LivePreview,
            history: Vec::new(),
            history_ix: 0,
            leaf: None,
            view_state: None,
        };
        let written = leaf_json(&tab);
        assert_eq!(written["state"]["type"], json!("pdf"));
        assert_eq!(written["state"]["icon"], json!("lucide-file-text"));
        assert_eq!(written["state"]["title"], json!("Field"));
        assert_eq!(
            written["state"]["state"],
            json!({"file": "Papers/Field.pdf"})
        );
        assert!(written["state"]["state"].get("mode").is_none());
        assert_eq!(tab.title(), "Field", "the strip shows no extension either");
        assert_eq!(tab_display_name("Field.PDF"), "Field");
        let known = |_: &str| true;
        let (_, restored) = tab_from_leaf(&written, ViewMode::Reading, &known).unwrap();
        assert_eq!(restored.view_state, None);
        assert_eq!(leaf_json(&restored), written);
        // A leaf somebody else wrote with keys of its own keeps them.
        let read = json!({
            "id": "0f6c3a9d2b1e4c5a",
            "type": "leaf",
            "state": {
                "type": "pdf",
                "state": {"file": "Papers/Field.pdf", "mystery": true},
                "icon": "lucide-file-text",
                "title": "Field"
            }
        });
        let (_, restored) = tab_from_leaf(&read, ViewMode::Reading, &known).unwrap();
        assert_eq!(leaf_json(&restored), read, "never moved: written as read");
    }

    #[test]
    fn a_tab_that_moved_to_another_note_gets_a_fresh_leaf() {
        let read = leaf("a1072f377b3d1c69", "Old.md", "source", false);
        let tab = Tab {
            path: Some("New.md".into()),
            mode: ViewMode::LivePreview,
            history: Vec::new(),
            history_ix: 0,
            leaf: Some(read),
            view_state: None,
        };
        let written = leaf_json(&tab);
        let id = written["id"].as_str().unwrap();
        assert_eq!(id.len(), 16);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(id, "a1072f377b3d1c69");
        assert_eq!(written["state"]["type"], json!("markdown"));
        assert_eq!(written["state"]["state"]["file"], json!("New.md"));
        assert_eq!(written["state"]["title"], json!("New"));
        assert!(written["state"].get("pinned").is_none());
    }

    #[test]
    fn an_empty_tab_and_a_graph_tab_write_their_own_leaf_types() {
        let empty = leaf_json(&Tab::empty(ViewMode::LivePreview));
        assert_eq!(empty["state"]["type"], json!("empty"));
        assert_eq!(empty["state"]["title"], json!("New tab"));
        let graph = leaf_json(&Tab::empty(ViewMode::Graph));
        assert_eq!(graph["state"]["type"], json!("graph"));
        assert_eq!(graph["state"]["icon"], json!("lucide-git-fork"));
    }

    /// An empty leaf is called "New tab" wherever its name is shown: the
    /// tab strip, the view header and the window title all ask for the one
    /// string, so none of them can read "No file is open" (see
    /// [[Specification#The empty tab]]).
    #[test]
    fn an_empty_leaf_is_called_new_tab_wherever_its_name_is_shown() {
        for mode in [
            ViewMode::Reading,
            ViewMode::LivePreview,
            ViewMode::Source,
            ViewMode::Reading,
        ] {
            assert_eq!(leaf_title(None, mode), "New tab");
            assert_eq!(Tab::empty(mode).title(), "New tab");
        }
        assert_eq!(leaf_title(None, ViewMode::Graph), "Graph view");
        assert_eq!(
            leaf_title(Some("Folder/Note.md"), ViewMode::LivePreview),
            "Note"
        );
    }

    /// The empty tab's four pills, in Obsidian's order and with the text it
    /// draws on them: the hotkeys come from the registry, so `New note` and
    /// `Go to file` carry the chords their commands are bound to and the
    /// other two carry none.
    #[test]
    fn the_empty_tab_offers_obsidians_four_pills() {
        let registry = CommandRegistry::with_known_ids();
        let actions = empty_tab_actions(&registry);
        let mod_key = if cfg!(target_os = "macos") {
            "Cmd"
        } else {
            "Ctrl"
        };
        let texts: Vec<String> = actions.iter().map(EmptyAction::text).collect();
        assert_eq!(
            texts,
            [
                format!("New note {mod_key} + N"),
                "New base".to_string(),
                "New canvas".to_string(),
                format!("Go to file {mod_key} + O"),
            ]
        );
        let commands: Vec<&str> = actions.iter().map(|a| a.command).collect();
        assert_eq!(
            commands,
            [
                "file-explorer:new-file",
                "bases:new-file",
                "canvas:new-file",
                "switcher:open"
            ]
        );
        // Each pill runs its command, so each has to be one the workspace
        // will actually run.
        for action in &actions {
            assert!(
                RUNNABLE.contains(&action.command),
                "{} is not runnable",
                action.command
            );
        }
    }

    /// A pill whose command the registry does not have is not drawn: the
    /// registry holds the commands of the core plugins that are on, so
    /// turning Bases off takes "New base" with it rather than leaving a
    /// pill that does nothing.
    #[test]
    fn a_pill_for_a_disabled_plugin_is_left_out() {
        let mut registry = CommandRegistry::with_known_ids();
        registry.retain_enabled(|plugin| plugin != "bases");
        let labels: Vec<&str> = empty_tab_actions(&registry)
            .iter()
            .map(|a| a.label)
            .collect();
        assert_eq!(labels, ["New note", "New canvas", "Go to file"]);
    }

    /// The status bar is not hidden on an empty tab: with one active
    /// Obsidian's reads `0 properties`, `0 words`, `0 characters`. A canvas
    /// and a base show nothing, and a note keeps the items it had.
    #[test]
    fn an_empty_tabs_status_bar_reads_zero_rather_than_nothing() {
        assert_eq!(
            Workspace::status_items(None, "", 0, 0, 0),
            ["0 properties", "0 words", "0 characters"]
        );
        assert!(Workspace::status_items(Some("Board.canvas"), "", 0, 0, 0).is_empty());
        assert!(Workspace::status_items(Some("Table.base"), "", 0, 0, 0).is_empty());
        assert_eq!(
            Workspace::status_items(Some("Note.md"), "Ln 1, Col 1  ·  ", 1, 12, 60),
            ["Ln 1, Col 1  ·  1 backlink  ·  12 words, 60 characters"]
        );
    }

    /// The empty tab's header carries the two navigation buttons and no
    /// mode switch: there is nothing to read or edit. The graph keeps its
    /// own toggle instead, as it already did.
    #[test]
    fn the_empty_tabs_header_has_no_mode_switch() {
        for mode in [
            ViewMode::Reading,
            ViewMode::LivePreview,
            ViewMode::Source,
            ViewMode::Graph,
        ] {
            assert!(
                !header_shows_mode_switch(&None, mode),
                "an empty tab in {mode:?} must not offer the mode switch"
            );
        }
        assert!(header_shows_mode_switch(
            &Some("Note.md".to_string()),
            ViewMode::Reading
        ));
        assert!(!header_shows_mode_switch(
            &Some("Note.md".to_string()),
            ViewMode::Graph
        ));
    }

    /// Reaching the empty tab by closing every tab gives the same view as
    /// a fresh vault's: [`Workspace::close_tab`] pushes a [`Tab::empty`] in
    /// the default mode, and every one of those is the same leaf -- no
    /// file, the "New tab" name, the empty main view and the zeroed status
    /// bar -- however the window got there.
    #[test]
    fn closing_every_tab_leaves_the_tab_a_fresh_vault_opens_with() {
        let fresh = Tab::empty(ViewMode::Reading);
        for mode in [ViewMode::LivePreview, ViewMode::Source, ViewMode::Reading] {
            let closed = Tab::empty(mode);
            assert_eq!(closed.path, fresh.path);
            assert_eq!(closed.title(), fresh.title());
            assert_eq!(
                Workspace::main_view_for(closed.mode, false, false, false, false, false, false),
                MainView::Empty
            );
            assert_eq!(
                Workspace::status_items(closed.path.as_deref(), "", 0, 0, 0),
                Workspace::status_items(fresh.path.as_deref(), "", 0, 0, 0)
            );
            assert!(!header_shows_mode_switch(&closed.path, closed.mode));
        }
    }

    /// Enter in the view-header title renames the file the field opened on,
    /// through the path the rename prompt's box commits through: the field
    /// holds the stem, so the extension goes back on and the name is
    /// checked before the filesystem is asked for it. The gesture the probe
    /// recorded: `Props` clicked at offset 3 and typed `X` reads `ProXps`
    /// while the file is still `Props.md`, and Enter asks for `ProXps.md`.
    #[test]
    fn the_header_titles_commit_asks_for_the_path_the_rename_prompt_would() {
        let mut field = TitleField::new("Props");
        field.click(3, 1, false);
        field.insert("X");
        assert_eq!(
            field.text(),
            "ProXps",
            "the title changed, the file has not"
        );
        assert_eq!(
            renamed_path("Props.md", field.text().trim(), false).as_deref(),
            Ok("ProXps.md")
        );
        assert_eq!(
            renamed_path("Folder/Props.md", "New name", false).as_deref(),
            Ok("Folder/New name.md")
        );
        assert_eq!(
            renamed_path("Folder/Board.canvas", "Board 2", false).as_deref(),
            Ok("Folder/Board 2.canvas")
        );
        // A folder has no extension to put back.
        assert_eq!(
            renamed_path("Folder", "Renamed", true).as_deref(),
            Ok("Renamed")
        );
        // And a name the filesystem would refuse never gets there: the
        // field stays open on the error instead.
        assert!(renamed_path("Props.md", "", false).is_err());
        assert!(renamed_path("Props.md", "a/b", false).is_err());
        assert!(renamed_path("Props.md", &"x".repeat(300), false).is_err());
    }

    /// Obsidian's `main` after "Split right" on A, then B opened in the new
    /// group and "Split down" there, as observed on 2026-10-20: a vertical
    /// root over a `tabs` group and a nested horizontal split.
    fn nested_main() -> Value {
        json!({
            "id": "449f821d63addc06",
            "type": "split",
            "children": [
                {"id": "3ab610226e243404", "type": "tabs", "dimension": 29.411764705882355,
                 "children": [leaf("3c245a8fc7dc4598", "A.md", "source", false)]},
                {"id": "0fa61223ab358f4f", "type": "split", "dimension": 70.58823529411765,
                 "children": [
                    {"id": "6c9c4444fe9200bb", "type": "tabs",
                     "children": [leaf("f0559f5cf8f3ac65", "B.md", "source", false)]},
                    {"id": "189916eb401608c6", "type": "tabs", "currentTab": 1,
                     "children": [
                        leaf("747a588e39ce11fa", "B.md", "source", false),
                        leaf("52ae6ba5c0105abd", "C.md", "preview", false),
                     ]},
                 ],
                 "direction": "horizontal"},
            ],
            "direction": "vertical",
        })
    }

    #[test]
    fn main_is_read_as_a_tree_of_groups_with_their_own_keys() {
        let spec = read_main(&nested_main(), ViewMode::LivePreview, &|_| true).unwrap();
        let LayoutSpec::Split {
            direction,
            children,
            node,
        } = spec
        else {
            panic!("the root is a split");
        };
        assert_eq!(direction, SplitDirection::Vertical);
        assert_eq!(node.get("id"), Some(&json!("449f821d63addc06")));
        assert!(!node.contains_key("children") && !node.contains_key("direction"));
        assert_eq!(children.len(), 2);
        let LayoutSpec::Group(left) = &children[0] else {
            panic!("the first child is a group");
        };
        assert_eq!(left.tabs.len(), 1);
        assert_eq!(left.tabs[0].0, "3c245a8fc7dc4598");
        assert_eq!(left.tabs[0].1.mode, ViewMode::LivePreview);
        assert_eq!(left.node.get("dimension"), Some(&json!(29.411764705882355)));
        let LayoutSpec::Split {
            direction,
            children,
            ..
        } = &children[1]
        else {
            panic!("the second child is a split");
        };
        assert_eq!(*direction, SplitDirection::Horizontal);
        let LayoutSpec::Group(bottom) = &children[1] else {
            panic!("a group");
        };
        assert_eq!(bottom.active_tab, 1);
        assert_eq!(bottom.tabs[1].1.mode, ViewMode::Reading);
    }

    #[test]
    fn a_group_whose_files_are_all_gone_is_dropped_with_its_empty_split() {
        let spec = read_main(&nested_main(), ViewMode::LivePreview, &|f| f == "A.md").unwrap();
        let LayoutSpec::Split { children, .. } = spec else {
            panic!("the root is a split");
        };
        assert_eq!(children.len(), 1, "the horizontal split had nothing left");
        assert!(read_main(&nested_main(), ViewMode::LivePreview, &|_| false).is_none());
    }

    #[test]
    fn the_top_right_group_follows_the_splits_shape() {
        // Side by side: the last child. Stacked: the first.
        let row = Layout::split(
            SplitDirection::Vertical,
            vec![Layout::Group(1), Layout::Group(2)],
        );
        assert_eq!(row.top_right(), Some(2));
        let stack = Layout::split(
            SplitDirection::Horizontal,
            vec![Layout::Group(1), Layout::Group(2)],
        );
        assert_eq!(stack.top_right(), Some(1));
        // A stack on the right of a row: its top group holds the corner,
        // though the depth-first order ends on its bottom one.
        let nested = Layout::split(
            SplitDirection::Vertical,
            vec![
                Layout::Group(1),
                Layout::split(
                    SplitDirection::Horizontal,
                    vec![Layout::Group(2), Layout::Group(3)],
                ),
            ],
        );
        assert_eq!(nested.top_right(), Some(2));
        let mut order = Vec::new();
        nested.groups(&mut order);
        assert_eq!(order.last(), Some(&3));
    }

    #[test]
    fn splitting_puts_a_sibling_in_a_split_that_runs_that_way_and_wraps_otherwise() {
        let mut tree = Layout::split(SplitDirection::Vertical, vec![Layout::Group(1)]);
        assert_eq!(
            tree.insert_beside(1, 2, SplitDirection::Vertical, true),
            Some(false)
        );
        let mut order = Vec::new();
        tree.groups(&mut order);
        assert_eq!(order, [1, 2]);
        assert_eq!(
            tree.insert_beside(2, 3, SplitDirection::Horizontal, true),
            Some(true)
        );
        assert_eq!(tree.path_to(3), Some(vec![1, 1]));
        assert!(matches!(
            tree.node_at(&[1]),
            Some(Layout::Split { direction: SplitDirection::Horizontal, children, .. }) if children.len() == 2
        ));
        // Neighbours: from 1, right is the whole right column; from 3, up is 2
        // and left is 1; from 2, down is 3, up is nothing.
        assert_eq!(tree.neighbors(1, SplitDirection::Vertical, true), [2, 3]);
        assert_eq!(tree.neighbors(3, SplitDirection::Horizontal, false), [2]);
        assert_eq!(tree.neighbors(3, SplitDirection::Vertical, false), [1]);
        assert_eq!(tree.neighbors(2, SplitDirection::Horizontal, true), [3]);
        assert!(tree
            .neighbors(2, SplitDirection::Horizontal, false)
            .is_empty());
        assert!(tree
            .neighbors(1, SplitDirection::Vertical, false)
            .is_empty());
        // Removing 2 leaves the horizontal split nested around 3, as Obsidian
        // keeps it; removing 3 too takes the split with it.
        assert!(tree.remove(2));
        assert_eq!(tree.path_to(3), Some(vec![1, 0]));
        assert!(tree.remove(3));
        let mut order = Vec::new();
        tree.groups(&mut order);
        assert_eq!(order, [1]);
        assert!(matches!(&tree, Layout::Split { children, .. } if children.len() == 1));
    }

    /// A tab dropped on a group's left or top edge goes before it: a sibling
    /// in front when the split runs that way, the first child of the split
    /// that wraps the pair when it does not.
    #[test]
    fn dropping_on_a_near_edge_puts_the_new_group_in_front() {
        let mut tree = Layout::split(SplitDirection::Vertical, vec![Layout::Group(1)]);
        assert_eq!(
            tree.insert_beside(1, 2, SplitDirection::Vertical, false),
            Some(false)
        );
        let mut order = Vec::new();
        tree.groups(&mut order);
        assert_eq!(order, [2, 1]);
        assert_eq!(
            tree.insert_beside(1, 3, SplitDirection::Horizontal, false),
            Some(true)
        );
        assert_eq!(tree.path_to(3), Some(vec![1, 0]));
        assert_eq!(tree.path_to(1), Some(vec![1, 1]));
        assert_eq!(tree.neighbors(1, SplitDirection::Horizontal, false), [3]);
    }

    /// The zones over a group's content: the outer band of each side splits
    /// towards that side, the corner goes to the nearer edge, and the middle
    /// joins the strip. The band is measured on the shorter side.
    #[test]
    fn drop_zones_follow_the_nearest_edge_within_the_band() {
        let bounds = gpui::Bounds {
            origin: point(px(100.), px(50.)),
            size: gpui::size(px(400.), px(200.)),
        };
        let at = |x: f32, y: f32| DropZone::at(bounds, point(px(100. + x), px(50. + y)));
        // The band is 30 percent of 200: 60 px.
        assert_eq!(at(200., 100.), DropZone::Center);
        assert_eq!(at(10., 100.), DropZone::Left);
        assert_eq!(at(390., 100.), DropZone::Right);
        assert_eq!(at(200., 10.), DropZone::Top);
        assert_eq!(at(200., 190.), DropZone::Bottom);
        assert_eq!(at(70., 100.), DropZone::Center);
        assert_eq!(at(200., 65.), DropZone::Center);
        // In a corner the nearer edge wins.
        assert_eq!(at(20., 40.), DropZone::Left);
        assert_eq!(at(40., 20.), DropZone::Top);
        assert_eq!(
            DropZone::Left.split(),
            Some((SplitDirection::Vertical, false))
        );
        assert_eq!(
            DropZone::Bottom.split(),
            Some((SplitDirection::Horizontal, true))
        );
        assert_eq!(DropZone::Center.split(), None);
    }

    #[test]
    fn a_dimension_sits_after_type() {
        let mut node = Map::new();
        node.insert("id".into(), json!("x"));
        node.insert("type".into(), json!("tabs"));
        node.insert("extra".into(), json!(1));
        set_dimension(&mut node, Some(40.0));
        let keys: Vec<&String> = node.keys().collect();
        assert_eq!(keys, ["id", "type", "dimension", "extra"]);
        set_dimension(&mut node, None);
        assert!(!node.contains_key("dimension"));
    }

    /// The Files leaf's state round trips through the two helpers: the
    /// keys keep their place among the ones Obsidian also writes, and a
    /// leaf written without a state gains one.
    #[test]
    fn explorer_leaf_state_is_read_and_written_in_place() {
        let mut layout = json!({
            "left": {"type": "split", "children": [{
                "type": "tabs",
                "children": [
                    {"type": "leaf", "state": {"type": "file-explorer", "state": {
                        "sortOrder": "byModifiedTime", "autoReveal": true,
                        "showSearch": false, "searchQuery": ""
                    }}},
                    {"type": "leaf", "state": {"type": "search"}},
                ],
            }]},
        });
        assert_eq!(
            explorer_leaf_state(&layout),
            Some((SortOrder::ByModifiedTime, true))
        );
        set_explorer_leaf_state(&mut layout["left"], SortOrder::ByCreatedTimeReverse, false);
        let state = &layout["left"]["children"][0]["children"][0]["state"]["state"];
        let keys: Vec<&String> = state.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            ["sortOrder", "autoReveal", "showSearch", "searchQuery"]
        );
        assert_eq!(state["sortOrder"], "byCreatedTimeReverse");
        assert_eq!(state["autoReveal"], false);

        let mut bare = json!({"left": {"type": "split", "children": [{
            "type": "tabs",
            "children": [{"type": "leaf", "state": {"type": "file-explorer"}}],
        }]}});
        assert_eq!(
            explorer_leaf_state(&bare),
            Some((SortOrder::Alphabetical, false))
        );
        set_explorer_leaf_state(&mut bare["left"], SortOrder::AlphabeticalReverse, true);
        assert_eq!(
            bare["left"]["children"][0]["children"][0]["state"]["state"],
            json!({"sortOrder": "alphabeticalReverse", "autoReveal": true})
        );

        let no_leaf = json!({"left": {"type": "split", "children": []}});
        assert_eq!(explorer_leaf_state(&no_leaf), None);
        let mut untouched = no_leaf.clone();
        set_explorer_leaf_state(&mut untouched["left"], SortOrder::ByCreatedTime, true);
        assert_eq!(untouched, no_leaf);
    }

    #[test]
    fn a_docks_current_tab_is_the_leaf_current_tab_points_at() {
        let layout = json!({
            "left": {"type": "split", "children": [{
                "type": "tabs",
                "currentTab": 1,
                "children": [
                    {"type": "leaf", "state": {"type": "file-explorer"}},
                    {"type": "leaf", "state": {"type": "search"}},
                ],
            }]},
            "right": {"type": "split", "children": [{
                "type": "tabs",
                "children": [{"type": "leaf", "state": {"type": "backlink"}}],
            }]},
        });
        assert_eq!(
            dock_current_view(&layout, "left").as_deref(),
            Some("search")
        );
        assert_eq!(
            dock_current_view(&layout, "right").as_deref(),
            Some("backlink")
        );
        assert_eq!(dock_current_view(&layout, "main"), None);
    }

    #[test]
    fn prune_history_forgets_a_deleted_note() {
        let mut h = history(&["A.md", "B.md", "C.md"]);
        assert_eq!(prune_history(&mut h, 2, "B.md"), 1);
        assert_eq!(h, history(&["A.md", "C.md"]));

        let mut h = history(&["A.md", "B.md", "C.md"]);
        assert_eq!(prune_history(&mut h, 3, "A.md"), 2, "still at C.md");
        assert_eq!(h, history(&["B.md", "C.md"]));

        let mut h = history(&["A.md", "B.md"]);
        assert_eq!(prune_history(&mut h, 1, "B.md"), 1, "after the cursor");
        assert_eq!(h, history(&["A.md"]));
    }

    /// Deleting a folder deletes every note under it, and a repeat visit is
    /// a repeat entry, so more than one entry can go at once.
    #[test]
    fn prune_history_forgets_a_whole_folder() {
        let mut h = history(&["Sub/S.md", "A.md", "Sub/T.md", "A.md"]);
        assert_eq!(prune_history(&mut h, 4, "Sub"), 2);
        assert_eq!(h, history(&["A.md", "A.md"]));

        let mut h = history(&["A.md", "A.md", "B.md"]);
        assert_eq!(prune_history(&mut h, 3, "A.md"), 1);
        assert_eq!(h, history(&["B.md"]));
    }

    /// A path the history never held, and a prefix that is not a folder
    /// boundary, leave it alone.
    #[test]
    fn prune_history_keeps_what_it_does_not_name() {
        let mut h = history(&["A.md", "Subtotal/S.md"]);
        assert_eq!(prune_history(&mut h, 2, "Sub"), 2);
        assert_eq!(h, history(&["A.md", "Subtotal/S.md"]));

        let mut h = history(&["A.md"]);
        assert_eq!(prune_history(&mut h, 0, "A.md"), 0, "nothing open");
        assert!(h.is_empty());
    }

    /// The path a toggle takes: the flipped config reaches `app.json` and
    /// the state in hand, and the key the vault already had is left alone.
    #[test]
    fn a_setting_change_writes_its_file_and_updates_the_state() {
        let (dir, state) = vault();
        assert_eq!(state.app().trash_option.as_deref(), Some("local"));
        assert_eq!(state.app().vim_mode, None);

        let mut app = state.app().clone();
        app.vim_mode = Some(true);
        state.apply_setting(&SettingChange::App(app)).unwrap();

        assert_eq!(state.app().vim_mode, Some(true), "the state follows");
        let text = fs::read_to_string(dir.path().join(".obsidian/app.json")).unwrap();
        assert_eq!(
            text, "{\n  \"trashOption\": \"local\",\n  \"vimMode\": true\n}",
            "the file kept what it had and gained one key"
        );
    }

    /// A config file the vault does not have yet is created, and only with
    /// what changed.
    #[test]
    fn a_setting_change_creates_a_missing_config_file() {
        let (dir, state) = vault();
        let mut switcher = state.switcher().clone();
        switcher.show_attachments = Some(false);
        state
            .apply_setting(&SettingChange::Switcher(switcher))
            .unwrap();
        let text = fs::read_to_string(dir.path().join(".obsidian/switcher.json")).unwrap();
        assert_eq!(text, "{\n  \"showAttachments\": false\n}");
    }

    #[test]
    fn a_hotkeys_change_is_written_whole_so_a_restored_default_leaves_the_file() {
        let (dir, state) = vault();
        let path = dir.path().join(".obsidian/hotkeys.json");
        fs::write(
            &path,
            "\u{feff}{\n  \"editor:toggle-bold\": [],\n  \"my-plugin:thing\": [\n    {\n      \"modifiers\": [\n        \"Alt\"\n      ],\n      \"key\": \"X\"\n    }\n  ]\n}",
        )
        .unwrap();
        let mut overrides = state.vault().hotkeys().unwrap();
        assert_eq!(overrides.len(), 2);
        overrides.shift_remove("editor:toggle-bold");
        state
            .apply_setting(&SettingChange::Hotkeys(overrides))
            .unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with('\u{feff}'), "the byte order mark stays");
        assert!(!text.contains("toggle-bold"), "{text}");
        assert!(text.contains("my-plugin:thing"), "{text}");
        assert_eq!(
            text.trim_start_matches('\u{feff}'),
            "{\n  \"my-plugin:thing\": [\n    {\n      \"modifiers\": [\n        \"Alt\"\n      ],\n      \"key\": \"X\"\n    }\n  ]\n}"
        );
    }

    /// The block id write: it reaches the target's bytes, keeps that
    /// note's own line endings, and refuses a file it did not plan from.
    #[test]
    fn writing_a_block_id_reaches_the_disk_and_refuses_a_stale_file() {
        use silica_suggest::blocks::{plan_id, Placement};
        let (dir, state) = vault();
        let path = "A.md";
        let before = fs::read_to_string(dir.path().join(path)).unwrap();
        let end = before.find("See [[B]]").unwrap() + "See [[B]] and [S](Sub/S.md).".len();
        let insertion = plan_id(&before, end, Placement::Inline, "abc123", "\n").expect("a plan");
        state.write_block_id(path, &before, &insertion).unwrap();
        let after = fs::read_to_string(dir.path().join(path)).unwrap();
        assert_eq!(after, insertion.applied(&before));
        assert!(
            after.contains("^abc123"),
            "the id is not in the file: {after}"
        );

        // The same insertion against bytes the file no longer holds is
        // refused rather than written.
        let err = state
            .write_block_id(path, &before, &insertion)
            .expect_err("the file changed since");
        assert!(err.contains("changed on disk"), "{err}");
        assert_eq!(
            fs::read_to_string(dir.path().join(path)).unwrap(),
            after,
            "a refused write still changed the file"
        );
    }

    /// A CRLF note keeps its own endings when an id is written into it,
    /// which is where Silica deliberately differs from Obsidian.
    #[test]
    fn a_block_id_written_into_a_crlf_note_keeps_its_endings() {
        use silica_suggest::blocks::{plan_id, Placement};
        let (dir, state) = vault();
        let path = "Crlf.md";
        let before = "# Crlf\r\n\r\npara in a CRLF file.\r\n";
        fs::write(dir.path().join(path), before).unwrap();
        let end = before.find("para in a CRLF file.").unwrap() + "para in a CRLF file.".len();
        let insertion = plan_id(before, end, Placement::Inline, "abc123", "\r\n").expect("a plan");
        state.write_block_id(path, before, &insertion).unwrap();
        let after = fs::read_to_string(dir.path().join(path)).unwrap();
        assert_eq!(after, "# Crlf\r\n\r\npara in a CRLF file. ^abc123\r\n");
        assert!(
            !after.contains("\n\n"),
            "a bare newline reached a CRLF note"
        );
    }

    /// A new base is a **zero-byte** file, and the second one is
    /// `Untitled 1.base`. Obsidian writes nothing into a base it creates --
    /// the parsed query of an empty file is already one default view -- so an
    /// untouched base of Silica's is byte-identical to an untouched base of
    /// Obsidian's. This is the pair of calls `new_base` makes; the command
    /// itself needs a window.
    #[test]
    fn a_new_base_is_an_empty_file_and_the_next_one_is_numbered() {
        let (dir, state) = vault();
        let first = state.vault().available_path("", "Untitled", "base");
        assert_eq!(first, "Untitled.base");
        state.create_note(&first, "").unwrap();
        assert_eq!(fs::read(dir.path().join(&first)).unwrap(), b"");
        assert_eq!(FileKind::from_path(&first), FileKind::Base);

        let second = state.vault().available_path("", "Untitled", "base");
        assert_eq!(second, "Untitled 1.base");
        state.create_note(&second, "").unwrap();
        assert_eq!(fs::read(dir.path().join(&second)).unwrap(), b"");
    }

    /// The ribbon's base button runs `bases:new-file`, so the id has to be
    /// one the workspace will actually run: a button bound to an id outside
    /// `RUNNABLE` is a button that does nothing.
    #[test]
    fn the_ribbon_base_button_runs_a_runnable_command() {
        assert!(RUNNABLE.contains(&"bases:new-file"));
        assert!(
            CommandRegistry::with_known_ids()
                .get("bases:new-file")
                .is_some(),
            "bases:new-file is not one of Obsidian's own command ids"
        );
    }

    /// Creating a daily note: the bytes on disk are the template expanded,
    /// the template itself is untouched, and a second run changes nothing.
    #[test]
    fn creating_a_daily_note_writes_the_template_and_nothing_else() {
        use silica_templates::{expand, Context, Stamp, DEFAULT_DATE_FORMAT, DEFAULT_TIME_FORMAT};
        let (dir, state) = vault();
        fs::create_dir_all(dir.path().join("Journal")).unwrap();
        let template = "# {{title}}\n\nWritten {{date}}.\n";
        fs::write(dir.path().join("Sub/Daily.md"), template).unwrap();
        let at = Stamp {
            year: 2026,
            month: 10,
            day: 18,
            hour: 9,
            minute: 7,
            second: 3,
        };
        let (path, exists) = state
            .vault()
            .daily_note("YYYY-MM-DD", "Journal", &at)
            .unwrap();
        assert_eq!((path.as_str(), exists), ("Journal/2026-10-18.md", false));
        let text = expand(
            template,
            &Context {
                at,
                date_format: DEFAULT_DATE_FORMAT,
                time_format: DEFAULT_TIME_FORMAT,
                title: "2026-10-18",
            },
        );
        state.create_note(&path, &text).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join(&path)).unwrap(),
            "# 2026-10-18\n\nWritten 2026-10-18.\n"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("Sub/Daily.md")).unwrap(),
            template,
            "the template itself was written to"
        );
        // Running again finds it rather than creating a second one.
        let (again, exists) = state
            .vault()
            .daily_note("YYYY-MM-DD", "Journal", &at)
            .unwrap();
        assert_eq!((again.as_str(), exists), ("Journal/2026-10-18.md", true));
    }

    /// A daily note whose folder is missing is refused, and nothing is
    /// created — Obsidian reports the same and writes nothing.
    #[test]
    fn a_daily_note_with_no_folder_writes_nothing() {
        use silica_templates::Stamp;
        let (dir, state) = vault();
        let at = Stamp {
            year: 2026,
            month: 10,
            day: 18,
            hour: 0,
            minute: 0,
            second: 0,
        };
        let err = state
            .vault()
            .daily_note("YYYY-MM-DD", "NoSuchFolder", &at)
            .unwrap_err();
        assert!(err.to_string().contains("not found"), "{err}");
        assert!(!dir.path().join("NoSuchFolder").exists());
    }

    #[test]
    fn rename_leaves_a_note_that_changed_underneath_alone() {
        // The rewrites are planned from the text the index holds. If the
        // other app saved one of those notes since, writing the plan would
        // throw that save away, so the note keeps its own bytes.
        let (dir, state) = vault();
        let theirs = "# B\n\nRewritten by the other app, with [[A]] still in it.\n";
        fs::write(dir.path().join("B.md"), theirs).unwrap();
        let outcome = state.rename("A.md", "Renamed A.md", true).unwrap();
        assert_eq!(outcome.stale, vec!["B.md".to_string()]);
        assert_eq!(
            fs::read_to_string(dir.path().join("B.md")).unwrap(),
            theirs,
            "the other app's save was overwritten"
        );
        // The notes that did not change are still rewritten.
        assert!(outcome.rewritten_files >= 1);
        let sub = fs::read_to_string(dir.path().join("Sub/S.md")).unwrap();
        assert!(
            sub.contains("Renamed A"),
            "an untouched note was not rewritten: {sub}"
        );
    }

    #[test]
    fn rename_rewrites_links_on_disk_and_in_the_index() {
        let (dir, state) = vault();
        let outcome = state.rename("A.md", "Renamed A.md", true).unwrap();
        assert_eq!(
            outcome.renamed,
            vec![("A.md".to_string(), "Renamed A.md".to_string())]
        );
        assert_eq!((outcome.rewritten_files, outcome.rewritten_links), (2, 4));
        let b = fs::read_to_string(dir.path().join("B.md")).unwrap();
        assert_eq!(
            b,
            "\u{feff}---\r\nrelated: \"[[Renamed A]]\"\r\n---\r\n[[Renamed A]] and [[Renamed A#A|the a]] and `[[A]]`\r\n",
            "the BOM and CRLF endings survive"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("Sub/S.md")).unwrap(),
            "[Renamed A](Renamed%20A.md) [[B]]\n"
        );
        assert!(dir.path().join("Renamed A.md").is_file());
        assert!(!dir.path().join("A.md").exists());
        {
            let cache = state.cache();
            assert!(cache.file("A.md").is_none());
            assert!(cache.file("Renamed A.md").is_some());
            // B's `related` property, its two body links, and S's markdown link.
            assert_eq!(cache.backlinks("Renamed A.md").len(), 4);
            assert!(cache.unresolved_links("B.md").is_none());
            assert_eq!(state.texts()["B.md"].as_str(), b);
        }
        assert!(outcome.changes.structural());
        // The watcher's events for the same paths find nothing new.
        let changes = state.apply(&[
            dir.path().join("A.md"),
            dir.path().join("Renamed A.md"),
            dir.path().join("B.md"),
        ]);
        assert!(changes
            .files
            .iter()
            .all(|c| matches!(c, FileChange::Modified(_))));
        assert_eq!(state.cache().backlinks("Renamed A.md").len(), 4);
    }

    #[test]
    fn rename_without_updating_links_leaves_them_dangling() {
        let (dir, state) = vault();
        let outcome = state.rename("A.md", "A2.md", false).unwrap();
        assert_eq!(outcome.rewritten_links, 0);
        assert!(fs::read_to_string(dir.path().join("B.md"))
            .unwrap()
            .contains("[[A]] and"));
        assert_eq!(
            state.cache().unresolved_links("B.md").unwrap().get("A"),
            Some(&3),
            "two body links and the frontmatter link"
        );
        assert!(
            state.rename("A2.md", "B.md", true).is_err(),
            "destination exists"
        );
        assert_eq!(
            state.rename_preview("B.md", "Sub/B.md"),
            (0, 0),
            "a unique name moving folders changes no shortest link"
        );
        assert_eq!(state.rename_preview("B.md", "Sub/Bee.md"), (2, 2));
    }

    /// The names the control socket calls the four modes by, both ways;
    /// `preview` is Obsidian's config value, not one of them.
    #[test]
    fn mode_names_round_trip() {
        for mode in [
            ViewMode::Reading,
            ViewMode::LivePreview,
            ViewMode::Source,
            ViewMode::Graph,
        ] {
            assert_eq!(ViewMode::parse(mode.as_str()), Some(mode));
        }
        assert_eq!(ViewMode::parse("preview"), None);
        assert_eq!(ViewMode::parse(""), None);
    }

    /// A write through the control socket can only land inside the vault
    /// folder, and the path it is given is usually one that does not exist
    /// yet, so the folders it goes through are what the check has to look
    /// at.
    #[test]
    fn vault_paths_cannot_leave_the_vault() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::create_dir(root.join("Sub")).unwrap();
        fs::write(root.join("A.md"), "a").unwrap();

        assert_eq!(vault_path_in(&root, "A.md").unwrap(), root.join("A.md"));
        assert_eq!(
            vault_path_in(&root, "Sub/New.md").unwrap(),
            root.join("Sub/New.md"),
            "a note that is not there yet, under a folder that is"
        );
        assert_eq!(vault_path_in(&root, "./A.md").unwrap(), root.join("A.md"));

        assert!(vault_path_in(&root, "../escape.md").is_err());
        assert!(vault_path_in(&root, "Sub/../../escape.md").is_err());
        assert!(vault_path_in(&root, "/etc/passwd").is_err());
        assert!(vault_path_in(&root, "").is_err(), "the folder itself");

        // A symlinked folder in the vault: the path is innocent, the folder
        // it goes through is not.
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("there.md"), "o").unwrap();
        if symlink(outside.path(), &root.join("linked")).is_none() {
            return;
        }
        assert!(vault_path_in(&root, "linked/new.md").is_err());
        assert!(vault_path_in(&root, "linked/there.md").is_err());

        // A dangling link resolves to nothing, and a write through it would
        // create the file wherever it points.
        if symlink(&outside.path().join("gone.md"), &root.join("dangling.md")).is_none() {
            return;
        }
        assert!(vault_path_in(&root, "dangling.md").is_err());
    }

    /// A file kind reaches the state dump in lower case.
    #[test]
    fn file_kinds_are_named_in_lower_case() {
        assert_eq!(kind_name(FileKind::Markdown), "markdown");
        assert_eq!(kind_name(FileKind::Other), "other");
    }

    #[test]
    fn create_duplicate_and_trash_keep_the_index_current() {
        let (dir, state) = vault();
        assert_eq!(
            state.new_note_path("Missing", Some("Sub/S.md")),
            "Missing.md"
        );
        assert_eq!(
            state.new_note_path("Deep/Also Missing", None),
            "Deep/Also Missing.md"
        );
        let changes = state.create_note("Deep/Also Missing.md", "").unwrap();
        assert_eq!(
            changes.files,
            vec![FileChange::Added("Deep/Also Missing.md".into())]
        );
        assert!(state.cache().file("Deep/Also Missing.md").is_some());
        assert!(state.vault().folders().contains(&"Deep".to_string()));
        let (copy, _) = state.duplicate("A.md").unwrap();
        assert_eq!(copy, "A 1.md");
        assert_eq!(
            fs::read_to_string(dir.path().join("A 1.md")).unwrap(),
            fs::read_to_string(dir.path().join("A.md")).unwrap()
        );
        let changes = state.trash("Sub").unwrap();
        assert_eq!(changes.removed_notes, vec!["Sub/S.md"]);
        assert!(dir.path().join(".trash/Sub/S.md").is_file());
        assert!(state.cache().file("Sub/S.md").is_none());
        assert_eq!(state.cache().backlinks("B.md").len(), 2, "A and its copy");
        assert!(state.vault().folders().iter().all(|f| f != "Sub"));
    }

    /// The socket's `focus` may not park the keyboard on a view the main
    /// area is not painting: that handle is in no frame's dispatch tree, and
    /// from there the app answers no key at all, the one that would undo it
    /// included.
    #[test]
    fn focus_refuses_a_view_the_main_area_is_not_painting() {
        assert_eq!(
            Workspace::focus_target("editor", MainView::Editor),
            Ok(Some(MainView::Editor))
        );
        assert_eq!(
            Workspace::focus_target("workspace", MainView::Empty),
            Ok(None),
            "the root div paints in every frame"
        );
        let err = Workspace::focus_target("reading", MainView::Editor).unwrap_err();
        assert!(err.contains("painting editor, not reading"), "{err}");
        let err = Workspace::focus_target("graph", MainView::Empty).unwrap_err();
        assert!(err.contains("painting nothing, not graph"), "{err}");
        assert_eq!(
            Workspace::focus_target("pane", MainView::Editor),
            Err("unknown view: pane".to_string())
        );
    }

    /// The socket's own name for the Bases view is `bases`, in all three
    /// places that name a view: the `state` dump's `focus.name` and the
    /// painted view, the `focus` method, and the `type` method -- which
    /// reaches nothing there, because a base has no field of its own.
    #[test]
    fn the_socket_calls_the_bases_view_bases() {
        assert_eq!(MainView::Bases.as_str(), "bases");
        assert_eq!(
            Workspace::focus_target("bases", MainView::Bases),
            Ok(Some(MainView::Bases))
        );
        let err = Workspace::focus_target("bases", MainView::Editor).unwrap_err();
        assert!(err.contains("painting editor, not bases"), "{err}");
        let err = Workspace::focus_target("editor", MainView::Bases).unwrap_err();
        assert!(err.contains("painting bases, not editor"), "{err}");
        assert_eq!(
            Workspace::typed_text_target("bases", MainView::Bases),
            "none"
        );
        assert_eq!(
            Workspace::typed_text_target("editor", MainView::Bases),
            "none",
            "the editor's handle can hold the focus under a base"
        );
    }

    /// The socket's name for the PDF view is `pdf`, the same three ways;
    /// its page box is a field of its own, so `type` reaches it the way it
    /// reaches any input.
    #[test]
    fn the_socket_calls_the_pdf_view_pdf() {
        assert_eq!(MainView::Pdf.as_str(), "pdf");
        assert_eq!(
            Workspace::focus_target("pdf", MainView::Pdf),
            Ok(Some(MainView::Pdf))
        );
        let err = Workspace::focus_target("pdf", MainView::Editor).unwrap_err();
        assert!(err.contains("painting editor, not pdf"), "{err}");
        assert_eq!(Workspace::typed_text_target("pdf", MainView::Pdf), "none");
    }

    /// An editor whose buffer names no file never gets the main area: on
    /// launch over a vault with nothing to open the buffer belongs to
    /// nothing, and anything typed into it would be saved nowhere.
    #[test]
    fn the_main_area_paints_no_editor_without_a_file_behind_it() {
        for mode in [ViewMode::LivePreview, ViewMode::Source] {
            assert_eq!(
                Workspace::main_view_for(mode, false, false, false, false, false, false),
                MainView::Empty,
                "{mode:?} with nothing open"
            );
            assert_eq!(
                Workspace::main_view_for(mode, true, false, true, false, false, false),
                MainView::Editor,
                "{mode:?} with a note open"
            );
            // The `--synthetic` spike hands the editor a note that is in no
            // vault: the buffer names a file, and that is what mounts it.
            assert_eq!(
                Workspace::main_view_for(mode, false, false, true, false, false, false),
                MainView::Editor,
                "{mode:?} with a buffer that belongs to no vault"
            );
            assert_eq!(
                Workspace::main_view_for(mode, true, true, true, false, false, false),
                MainView::Reading,
                "{mode:?} over an image"
            );
        }
        assert_eq!(
            Workspace::main_view_for(ViewMode::Reading, false, false, true, false, false, false),
            MainView::Empty,
            "a buffer is not a built note"
        );
        assert_eq!(
            Workspace::main_view_for(ViewMode::Graph, false, false, false, false, false, false),
            MainView::Graph,
            "the graph needs no file"
        );
        for mode in [ViewMode::Reading, ViewMode::LivePreview, ViewMode::Source] {
            assert_eq!(
                Workspace::main_view_for(mode, false, false, false, true, false, false),
                MainView::Canvas,
                "{mode:?} over a canvas"
            );
            assert_eq!(
                Workspace::main_view_for(mode, false, false, false, false, true, false),
                MainView::Bases,
                "{mode:?} over a base"
            );
            assert_eq!(
                Workspace::main_view_for(mode, false, false, false, false, false, true),
                MainView::Pdf,
                "{mode:?} over a PDF"
            );
        }
        assert_eq!(
            Workspace::main_view_for(ViewMode::Graph, false, false, false, false, false, true),
            MainView::Graph,
            "the graph wins over a PDF"
        );
        // The graph is the one view that outranks a file: a canvas or a base
        // open behind it stays behind it.
        assert_eq!(
            Workspace::main_view_for(ViewMode::Graph, false, false, false, false, true, false),
            MainView::Graph,
            "the graph wins over a base"
        );
    }

    /// The socket's `type` reaches the editor only while the main area is
    /// painting it. The editor's handle can still hold the focus under the
    /// empty state, and this route does not go through the dispatch tree
    /// that would otherwise refuse it.
    #[test]
    fn typing_reaches_no_editor_the_main_area_is_not_painting() {
        for focus in ["editor", "property-field", "cell-field"] {
            assert_eq!(
                Workspace::typed_text_target(focus, MainView::Editor),
                "editor"
            );
            assert_eq!(Workspace::typed_text_target(focus, MainView::Empty), "none");
            assert_eq!(
                Workspace::typed_text_target(focus, MainView::Reading),
                "none"
            );
        }
        assert_eq!(
            Workspace::typed_text_target("prompt-input", MainView::Empty),
            "input",
            "a modal's field is on screen whatever the main area paints"
        );
        assert_eq!(
            Workspace::typed_text_target("workspace", MainView::Editor),
            "none"
        );
    }

    /// Outline rows carry the byte offsets of the text they were built from.
    /// The index is a save behind while a note is being typed, so rows built
    /// from it point into a version that is not on screen — typing a heading
    /// above another moves every offset below it, and a row clicked with the
    /// old offset lands somewhere else in the note.
    #[test]
    fn outline_rows_carry_the_offsets_of_the_text_they_were_built_from() {
        let saved = "# A\n\n### B\n\n## C\n";
        let typed = "## Typed heading\n\n# A\n\n### B\n\n## C\n";
        let flat = |rows: &[panes::Heading]| -> Vec<(u8, String, usize)> {
            rows.iter()
                .map(|r| (r.level, r.text.to_string(), r.offset))
                .collect()
        };
        assert_eq!(
            flat(&outline_headings(&index_markdown(saved))),
            vec![
                (1, "A".to_string(), saved.find("# A").unwrap()),
                (3, "B".to_string(), saved.find("### B").unwrap()),
                (2, "C".to_string(), saved.find("## C").unwrap()),
            ]
        );
        let buffer = outline_headings(&index_markdown(typed));
        assert_eq!(
            flat(&buffer),
            vec![
                (2, "Typed heading".to_string(), 0),
                (1, "A".to_string(), typed.find("# A").unwrap()),
                (3, "B".to_string(), typed.find("### B").unwrap()),
                (2, "C".to_string(), typed.find("## C").unwrap()),
            ]
        );
        for row in &buffer {
            assert!(
                typed[row.offset..].starts_with('#'),
                "row {:?} points at {:?}",
                row.text,
                &typed[row.offset..]
            );
        }
    }

    /// A notice leaves on its own timer, and only the one that timer named:
    /// a message put up later stays until its own turn comes.
    #[test]
    fn a_notice_expires_by_id_and_leaves_the_newer_one() {
        let mut n = Notices::default();
        let first = n.push("The cursor is not in a table".into());
        let second = n.push("A.md saved".into());
        assert_eq!(n.latest().map(|t| t.as_ref()), Some("A.md saved"));

        n.expire(first);
        assert_eq!(n.texts(), vec!["A.md saved".to_string()]);
        n.expire(second);
        assert!(n.texts().is_empty());
        assert!(n.latest().is_none(), "nothing is left to misread");
    }

    /// The autosave retries on a timer, so the same refusal arrives again
    /// and again: it renews the message on screen instead of stacking
    /// copies of it, and the renewed one outlives the first one's timer.
    #[test]
    fn a_repeated_notice_is_renewed_rather_than_stacked() {
        let mut n = Notices::default();
        let first = n.push("A.md changed on disk".into());
        let again = n.push("A.md changed on disk".into());
        assert_ne!(first, again);
        assert_eq!(n.texts(), vec!["A.md changed on disk".to_string()]);

        n.expire(first);
        assert_eq!(
            n.texts(),
            vec!["A.md changed on disk".to_string()],
            "the first timer must not take the renewed message down"
        );
        n.expire(again);
        assert!(n.texts().is_empty());
    }

    /// Clearing takes down everything, which is what the control socket's
    /// `notice_clear` is for.
    #[test]
    fn clearing_takes_every_notice_down() {
        let mut n = Notices::default();
        n.push("one".into());
        n.push("two".into());
        n.clear();
        assert!(n.latest().is_none());
        assert_eq!(n.push("three".into()), 3, "ids keep counting up");
    }
}
