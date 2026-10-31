//! The Files view: a folder tree over the vault's file list in the left dock.
//!
//! [`FileTree`] is the pure model (ordering, collapse state, visible rows)
//! and [`FileExplorer`] the GPUI entity that draws it with a uniform list, so
//! a 20k-file vault costs one row element per visible line. Names are ordered
//! with [`natural_cmp`], the collation the search pane uses, so two lists of
//! the same files in one window cannot disagree, and the files of each
//! folder follow the leaf's [`SortOrder`]. A right click opens the file menu
//! (new note, new folder, rename, move, copy, delete); the explorer only
//! reports the choice, the workspace does the work. Above the tree sits
//! Obsidian's `nav-header`: New note, New folder, Change sort order,
//! Auto-reveal current file, and Expand all (Collapse all once a folder is
//! open).
//!
//! The tree takes the keyboard too, under the "Explorer" key context: one
//! row carries the focus ring, Up and Down walk the visible rows, Right
//! opens a folder or steps into it, Left closes one or climbs to the parent,
//! Enter opens a file or toggles a folder, F2 and Delete act on the row,
//! and Escape hands the keyboard back to the main area — what the
//! 2026-10-21 `explorer-keyboard` probe saw Obsidian do.

use crate::assets::icon;
use crate::canvas::menu::tooltip;
use crate::theme::{Theme, ICON_SIZE};
use gpui::{
    actions, anchored, deferred, div, point, prelude::*, px, uniform_list, App, ClickEvent,
    Context, EventEmitter, FocusHandle, Focusable, FontWeight, KeyBinding, MouseButton, Pixels,
    Point, ScrollStrategy, SharedString, UniformListScrollHandle, Window,
};
use silica_formats::FileKind;
use silica_search::natural_cmp;
use silica_vault::VaultFile;
use serde_json::{json, Value};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};
use std::ops::Range;

/// The font size of a row's extension tag.
const TAG_SIZE: f32 = 10.0;

actions!(
    explorer,
    [
        FocusUp,
        FocusDown,
        FocusLeft,
        FocusRight,
        Activate,
        ToggleFolder,
        LeaveTree,
        RenameFocused,
        DeleteFocused
    ]
);

/// Key bindings for the tree; called once at startup. The context is the
/// explorer's own, beneath the workspace's.
pub fn bind_keys(cx: &mut App) {
    let ctx = Some("Explorer");
    cx.bind_keys([
        KeyBinding::new("up", FocusUp, ctx),
        KeyBinding::new("down", FocusDown, ctx),
        KeyBinding::new("left", FocusLeft, ctx),
        KeyBinding::new("right", FocusRight, ctx),
        KeyBinding::new("enter", Activate, ctx),
        KeyBinding::new("space", ToggleFolder, ctx),
        KeyBinding::new("escape", LeaveTree, ctx),
        KeyBinding::new("f2", RenameFocused, ctx),
        KeyBinding::new("delete", DeleteFocused, ctx),
    ]);
}

/// The header row: Obsidian's `.nav-header` is 40 px, 8 px of padding
/// around a centred row of 28 by 24 buttons 2 px apart.
const HEADER_HEIGHT: f32 = 40.0;

/// The order the files of each folder are listed in: the six values of the
/// Files leaf's `sortOrder`. Folders keep their name order whichever is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortOrder {
    #[default]
    Alphabetical,
    AlphabeticalReverse,
    ByModifiedTime,
    ByModifiedTimeReverse,
    ByCreatedTime,
    ByCreatedTimeReverse,
}

impl SortOrder {
    /// Every order, in the order the menu lists them.
    pub const ALL: [SortOrder; 6] = [
        SortOrder::Alphabetical,
        SortOrder::AlphabeticalReverse,
        SortOrder::ByModifiedTime,
        SortOrder::ByModifiedTimeReverse,
        SortOrder::ByCreatedTime,
        SortOrder::ByCreatedTimeReverse,
    ];

    /// The string `workspace.json` records.
    pub fn as_str(self) -> &'static str {
        match self {
            SortOrder::Alphabetical => "alphabetical",
            SortOrder::AlphabeticalReverse => "alphabeticalReverse",
            SortOrder::ByModifiedTime => "byModifiedTime",
            SortOrder::ByModifiedTimeReverse => "byModifiedTimeReverse",
            SortOrder::ByCreatedTime => "byCreatedTime",
            SortOrder::ByCreatedTimeReverse => "byCreatedTimeReverse",
        }
    }

    pub fn parse(s: &str) -> Option<SortOrder> {
        SortOrder::ALL.into_iter().find(|o| o.as_str() == s)
    }

    /// The menu's label.
    pub fn label(self) -> &'static str {
        match self {
            SortOrder::Alphabetical => "File name (A to Z)",
            SortOrder::AlphabeticalReverse => "File name (Z to A)",
            SortOrder::ByModifiedTime => "Modified time (new to old)",
            SortOrder::ByModifiedTimeReverse => "Modified time (old to new)",
            SortOrder::ByCreatedTime => "Created time (new to old)",
            SortOrder::ByCreatedTimeReverse => "Created time (old to new)",
        }
    }

    /// How two files of one folder compare under this order. Names tie
    /// break every time sort, and the full file name breaks a tie between
    /// two display names, so the order of a rescan is stable.
    fn compare(self, a: &Entry, b: &Entry) -> Ordering {
        let by_name = |a: &Entry, b: &Entry| {
            natural_cmp(display_name(&a.name), display_name(&b.name))
                .then_with(|| natural_cmp(&a.name, &b.name))
        };
        match self {
            SortOrder::Alphabetical => by_name(a, b),
            SortOrder::AlphabeticalReverse => {
                natural_cmp(display_name(&b.name), display_name(&a.name))
                    .then_with(|| natural_cmp(&a.name, &b.name))
            }
            SortOrder::ByModifiedTime => b.mtime.cmp(&a.mtime).then_with(|| by_name(a, b)),
            SortOrder::ByModifiedTimeReverse => a.mtime.cmp(&b.mtime).then_with(|| by_name(a, b)),
            SortOrder::ByCreatedTime => b.ctime.cmp(&a.ctime).then_with(|| by_name(a, b)),
            SortOrder::ByCreatedTimeReverse => a.ctime.cmp(&b.ctime).then_with(|| by_name(a, b)),
        }
    }
}

/// One line of the tree with every folder expanded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Vault-relative path; folders without a trailing slash.
    pub path: String,
    /// Name as shown: a file loses its extension, which `tag` carries.
    pub name: String,
    /// The extension of a file that is not Markdown, upper-cased at the
    /// row's right end the way Obsidian's `nav-file-tag` shows it.
    pub tag: Option<String>,
    pub depth: usize,
    pub is_folder: bool,
    pub kind: FileKind,
}

/// A file as its folder holds it: what the sort orders need of it.
#[derive(Debug, Clone)]
struct Entry {
    name: String,
    kind: FileKind,
    mtime: u64,
    ctime: u64,
}

#[derive(Debug, Default)]
struct Folder {
    folders: BTreeMap<String, Folder>,
    files: Vec<Entry>,
}

/// Folder tree with per-folder collapse state.
#[derive(Debug, Default)]
pub struct FileTree {
    root: Folder,
    sort: SortOrder,
    all: Vec<Row>,
    collapsed: HashSet<String>,
    visible: Vec<usize>,
}

impl FileTree {
    /// Builds the tree. Files of unknown kind are hidden unless
    /// `show_unsupported` (Obsidian's "Detect all file extensions");
    /// `folders` adds the empty folders the files alone would not show.
    pub fn new<'a>(
        files: impl IntoIterator<Item = &'a VaultFile>,
        folders: impl IntoIterator<Item = &'a String>,
        show_unsupported: bool,
    ) -> FileTree {
        let mut root = Folder::default();
        for path in folders {
            let mut folder = &mut root;
            for part in path.split('/').filter(|p| !p.is_empty()) {
                folder = folder.folders.entry(part.to_string()).or_default();
            }
        }
        for file in files {
            if file.kind == FileKind::Other && !show_unsupported {
                continue;
            }
            let mut parts: Vec<&str> = file.path.split('/').collect();
            let name = parts.pop().unwrap_or_default().to_string();
            let mut folder = &mut root;
            for part in parts {
                folder = folder.folders.entry(part.to_string()).or_default();
            }
            folder.files.push(Entry {
                name,
                kind: file.kind,
                mtime: file.mtime,
                ctime: file.ctime,
            });
        }
        let mut all = Vec::new();
        flatten(&root, "", 0, SortOrder::Alphabetical, &mut all);
        let collapsed = all
            .iter()
            .filter(|r| r.is_folder)
            .map(|r| r.path.clone())
            .collect();
        let mut tree = FileTree {
            root,
            sort: SortOrder::Alphabetical,
            all,
            collapsed,
            visible: Vec::new(),
        };
        tree.recompute();
        tree
    }

    pub fn sort(&self) -> SortOrder {
        self.sort
    }

    /// Re-lists every folder's files in `sort`; folders and their collapse
    /// state stay as they were.
    pub fn set_sort(&mut self, sort: SortOrder) {
        if self.sort == sort {
            return;
        }
        self.sort = sort;
        self.all.clear();
        flatten(&self.root, "", 0, sort, &mut self.all);
        self.recompute();
    }

    fn recompute(&mut self) {
        self.visible.clear();
        let mut hide_below: Option<usize> = None;
        for (ix, row) in self.all.iter().enumerate() {
            if let Some(depth) = hide_below {
                if row.depth > depth {
                    continue;
                }
                hide_below = None;
            }
            self.visible.push(ix);
            if row.is_folder && self.collapsed.contains(&row.path) {
                hide_below = Some(row.depth);
            }
        }
    }

    /// Rows currently shown, in order.
    pub fn rows(&self) -> impl ExactSizeIterator<Item = &Row> {
        self.visible.iter().map(|&ix| &self.all[ix])
    }

    pub fn row(&self, visible_ix: usize) -> Option<&Row> {
        self.visible.get(visible_ix).map(|&ix| &self.all[ix])
    }

    pub fn len(&self) -> usize {
        self.visible.len()
    }

    /// Folders currently expanded, to carry over to a rebuilt tree.
    pub fn expanded_folders(&self) -> HashSet<String> {
        self.all
            .iter()
            .filter(|r| r.is_folder && !self.collapsed.contains(&r.path))
            .map(|r| r.path.clone())
            .collect()
    }

    /// Expands the given folders (unknown ones are ignored).
    pub fn expand_folders(&mut self, folders: &HashSet<String>) {
        self.collapsed.retain(|f| !folders.contains(f));
        self.recompute();
    }

    pub fn is_collapsed(&self, folder: &str) -> bool {
        self.collapsed.contains(folder)
    }

    pub fn toggle(&mut self, folder: &str) {
        if !self.collapsed.remove(folder) {
            self.collapsed.insert(folder.to_string());
        }
        self.recompute();
    }

    pub fn collapse_all(&mut self) {
        self.collapsed = self
            .all
            .iter()
            .filter(|r| r.is_folder)
            .map(|r| r.path.clone())
            .collect();
        self.recompute();
    }

    pub fn expand_all(&mut self) {
        self.collapsed.clear();
        self.recompute();
    }

    /// Whether any folder is open: the header's last button is "Collapse
    /// all" then and "Expand all" otherwise, as Obsidian's swaps.
    pub fn any_expanded(&self) -> bool {
        self.all
            .iter()
            .any(|r| r.is_folder && !self.collapsed.contains(&r.path))
    }

    /// Expands every ancestor of `path` and returns its visible row index.
    pub fn reveal(&mut self, path: &str) -> Option<usize> {
        let mut prefix = String::new();
        for part in path.split('/') {
            if !prefix.is_empty() {
                self.collapsed.remove(&prefix);
                prefix.push('/');
            }
            prefix.push_str(part);
        }
        self.recompute();
        self.visible_index(path)
    }

    pub fn visible_index(&self, path: &str) -> Option<usize> {
        self.visible
            .iter()
            .position(|&ix| self.all[ix].path == path)
    }
}

fn flatten(folder: &Folder, prefix: &str, depth: usize, sort: SortOrder, out: &mut Vec<Row>) {
    let mut names: Vec<&String> = folder.folders.keys().collect();
    names.sort_by(|a, b| natural_cmp(a, b));
    for name in names {
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        out.push(Row {
            path: path.clone(),
            name: name.clone(),
            depth,
            tag: None,
            is_folder: true,
            kind: FileKind::Other,
        });
        flatten(&folder.folders[name], &path, depth + 1, sort, out);
    }
    let mut files: Vec<&Entry> = folder.files.iter().collect();
    files.sort_by(|a, b| sort.compare(a, b));
    for entry in files {
        let name = &entry.name;
        out.push(Row {
            path: if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            },
            name: stem(name).to_string(),
            tag: file_tag(name),
            depth,
            is_folder: false,
            kind: entry.kind,
        });
    }
}

/// The name a [`FileKind`] goes by outside the app: in the control socket's
/// state dump, and anywhere else a kind has to survive as text.
pub fn kind_name(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Markdown => "markdown",
        FileKind::Canvas => "canvas",
        FileKind::Base => "base",
        FileKind::Image => "image",
        FileKind::Audio => "audio",
        FileKind::Video => "video",
        FileKind::Pdf => "pdf",
        FileKind::Other => "other",
    }
}

/// A file's name without its extension, for the explorer's rows: Obsidian
/// shows every file bare and tags the ones that are not Markdown.
fn stem(file_name: &str) -> &str {
    match file_name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => stem,
        _ => file_name,
    }
}

/// The row's tag for a file that is not Markdown: its extension in upper
/// case (`CANVAS`, `PNG`); none for a note or a file without one.
fn file_tag(file_name: &str) -> Option<String> {
    match file_name.rsplit_once('.') {
        Some((stem, ext))
            if !stem.is_empty() && !ext.is_empty() && !ext.eq_ignore_ascii_case("md") =>
        {
            Some(ext.to_ascii_uppercase())
        }
        _ => None,
    }
}

/// Markdown files show without their extension, everything else with it.
pub fn display_name(file_name: &str) -> &str {
    match file_name.rsplit_once('.') {
        Some((stem, ext)) if ext.eq_ignore_ascii_case("md") && !stem.is_empty() => stem,
        _ => file_name,
    }
}

pub enum ExplorerEvent {
    Open(String),
    /// A Ctrl+click: the file goes to a new tab.
    OpenInNewTab(String),
    /// The header's "New note": what the `file-explorer:new-file` command
    /// does, the folder following `newFileLocation`.
    NewNoteFromHeader,
    /// Context-menu actions; paths are vault-relative, folders without a
    /// trailing slash (`""` is the vault root). The header's "New folder"
    /// is a `NewFolder` at the root.
    NewNote {
        folder: String,
    },
    NewFolder {
        parent: String,
    },
    /// "New canvas" and "New base" from a folder row, which take the folder
    /// the click was on rather than the vault root the commands use.
    NewCanvas {
        folder: String,
    },
    NewBase {
        folder: String,
    },
    Rename(String),
    Move(String),
    Duplicate(String),
    Delete(String),
    /// "Bookmark...": a file or folder for the Bookmarks pane.
    Bookmark(String),
    /// "Open to the right": a new tab group beside the active one showing
    /// this file.
    OpenToRight(String),
    /// "Copy path": the vault-relative path onto the clipboard.
    CopyPath(String),
    /// "Open in default app": hand the file to the desktop.
    OpenInDefaultApp(String),
    /// "Search in folder": the Search pane scoped to this folder.
    SearchInFolder(String),
    /// Escape in the tree: the keyboard goes back to the main area.
    LeaveFocus,
}

/// Obsidian's `data-section` on a menu item. Items are grouped by it in the
/// order below, and a separator is drawn wherever the section changes — which
/// is how the captured menus get their separators without any being written
/// down one at a time ([[Specification#The file explorer's context menus]]).
/// [`Section::Danger`] takes **two**: Obsidian emits an empty section before
/// it, and both menus were captured with the two consecutive separators that
/// leaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    /// The folder row's and the empty space's creation items.
    ActionPrimary,
    /// The file row's "Open in..." items.
    Open,
    Action,
    /// Obsidian leaves "Copy path" with no section at all.
    Unsectioned,
    System,
    Danger,
}

/// One row of a context menu: what Obsidian draws, and what Silica emits when
/// it is clicked.
struct MenuItem {
    label: &'static str,
    icon: &'static str,
    section: Section,
    event: ExplorerEvent,
}

/// What was right-clicked.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    /// A file row, by vault-relative path.
    File(String),
    /// A folder row; `""` is the vault root.
    Folder(String),
    /// The empty space below the last row, which Obsidian gives the four
    /// `action-primary` items alone, rooted at the vault.
    Blank,
}

/// The menu for a target, in Obsidian's order.
///
/// Four of the captured items are left out rather than drawn dead, because
/// Silica has no command behind them: **Open in new window** (no second
/// window), **Merge entire file with...** (no merge), **Open version history**
/// (no file history) and **Show in system explorer** (selecting a file in the
/// desktop's file manager wants the D-Bus `FileManager1.ShowItems` call). A
/// folder's **Make a copy** is out for the same reason: `Vault::duplicate`
/// copies one file, and a folder needs a recursive copy with link rewriting.
/// Listing a row that cannot run is what the command palette refuses to do,
/// and the menu refuses it too; the gaps are entries in
/// `docs/silica/UI parity backlog.md`. Dropping the only item of a section
/// drops its separator with it, so the `view` section's separator is absent
/// here where the capture has it.
///
/// Obsidian's file row has no creation items at all — Silica's menu used to
/// offer "New note" and "New folder" on a file, which is the one thing here
/// that was wrong rather than missing.
fn menu_items(target: &Target) -> Vec<MenuItem> {
    use ExplorerEvent as E;
    use Section as S;
    let item = |label, icon, section, event| MenuItem {
        label,
        icon,
        section,
        event,
    };
    match target {
        Target::File(path) => vec![
            item(
                "Open in new tab",
                "file-plus",
                S::Open,
                E::OpenInNewTab(path.clone()),
            ),
            item(
                "Open to the right",
                "separator-vertical",
                S::Open,
                E::OpenToRight(path.clone()),
            ),
            item(
                "Make a copy",
                "files",
                S::Action,
                E::Duplicate(path.clone()),
            ),
            item(
                "Move file to...",
                "folder-tree",
                S::Action,
                E::Move(path.clone()),
            ),
            item(
                "Bookmark...",
                "bookmark",
                S::Action,
                E::Bookmark(path.clone()),
            ),
            item(
                "Copy path",
                "clipboard",
                S::Unsectioned,
                E::CopyPath(path.clone()),
            ),
            item(
                "Open in default app",
                "arrow-up-right",
                S::System,
                E::OpenInDefaultApp(path.clone()),
            ),
            item("Rename...", "pen-line", S::Danger, E::Rename(path.clone())),
            item("Delete", "trash-2", S::Danger, E::Delete(path.clone())),
        ],
        Target::Folder(folder) => vec![
            // Obsidian draws `lucide-edit` here, which is the current
            // `square-pen`; `lucide-edit-3` on Rename is `pen-line`. Both
            // renames come from Lucide's own alias data, not a guess.
            item(
                "New note",
                "square-pen",
                S::ActionPrimary,
                E::NewNote {
                    folder: folder.clone(),
                },
            ),
            item(
                "New folder",
                "folder-open",
                S::ActionPrimary,
                E::NewFolder {
                    parent: folder.clone(),
                },
            ),
            item(
                "New canvas",
                "layout-dashboard",
                S::ActionPrimary,
                E::NewCanvas {
                    folder: folder.clone(),
                },
            ),
            item(
                "New base",
                "layout-list",
                S::ActionPrimary,
                E::NewBase {
                    folder: folder.clone(),
                },
            ),
            item(
                "Move folder to...",
                "folder-tree",
                S::Action,
                E::Move(folder.clone()),
            ),
            item(
                "Search in folder",
                "folder-search",
                S::Action,
                E::SearchInFolder(folder.clone()),
            ),
            item(
                "Bookmark...",
                "bookmark",
                S::Action,
                E::Bookmark(folder.clone()),
            ),
            item(
                "Copy path",
                "clipboard",
                S::Unsectioned,
                E::CopyPath(folder.clone()),
            ),
            item(
                "Rename...",
                "pen-line",
                S::Danger,
                E::Rename(folder.clone()),
            ),
            item("Delete", "trash-2", S::Danger, E::Delete(folder.clone())),
        ],
        Target::Blank => vec![
            item(
                "New note",
                "square-pen",
                S::ActionPrimary,
                E::NewNote {
                    folder: String::new(),
                },
            ),
            item(
                "New folder",
                "folder-open",
                S::ActionPrimary,
                E::NewFolder {
                    parent: String::new(),
                },
            ),
            item(
                "New canvas",
                "layout-dashboard",
                S::ActionPrimary,
                E::NewCanvas {
                    folder: String::new(),
                },
            ),
            item(
                "New base",
                "layout-list",
                S::ActionPrimary,
                E::NewBase {
                    folder: String::new(),
                },
            ),
        ],
    }
}

/// How many separators sit above `item` given the section above it: one
/// wherever the section changes, and two above [`Section::Danger`] for the
/// empty section Obsidian puts there.
fn separators_above(previous: Option<Section>, section: Section) -> usize {
    match previous {
        None => 0,
        Some(p) if p == section => 0,
        Some(_) if section == Section::Danger => 2,
        Some(_) => 1,
    }
}

/// The right-click menu, over a row or over the empty space below them.
struct Menu {
    position: Point<Pixels>,
    target: Target,
}

pub struct FileExplorer {
    tree: FileTree,
    active: Option<String>,
    /// The leaf's `autoReveal`: whether opening a file expands its folders
    /// and scrolls to its row, or only highlights the row if it is on
    /// screen. Off in a fresh vault, as in Obsidian.
    auto_reveal: bool,
    scroll: UniformListScrollHandle,
    menu: Option<Menu>,
    /// The sort menu is open under its header button.
    sort_menu_open: bool,
    focus_handle: FocusHandle,
    /// The row the keyboard is on (Obsidian's `has-focus`), by path so it
    /// survives a re-sort; it may be hidden under a collapsed folder, and
    /// then the next key starts over from the open file.
    focused: Option<String>,
}

impl Focusable for FileExplorer {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<ExplorerEvent> for FileExplorer {}

impl FileExplorer {
    pub fn new(cx: &mut Context<Self>) -> FileExplorer {
        FileExplorer {
            tree: FileTree::default(),
            active: None,
            auto_reveal: false,
            scroll: UniformListScrollHandle::new(),
            menu: None,
            sort_menu_open: false,
            focus_handle: cx.focus_handle(),
            focused: None,
        }
    }

    /// Gives the tree the keyboard, on the row it had or none: what
    /// `file-explorer:open` does.
    pub fn focus_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// Expands to `path`, puts the focus ring on its row and gives the tree
    /// the keyboard: `file-explorer:reveal-active-file`.
    pub fn reveal_and_focus(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.reveal(path, cx);
        self.focused = Some(path.to_string());
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// The visible index of the focused row, if it is on a visible row.
    fn focused_ix(&self) -> Option<usize> {
        self.tree.visible_index(self.focused.as_deref()?)
    }

    fn set_focused_ix(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(row) = self.tree.row(ix) {
            self.focused = Some(row.path.clone());
            self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
            cx.notify();
        }
    }

    /// Where Up and Down start from when no visible row has the ring: the
    /// open file's row if a folder is not hiding it, else the first row.
    fn start_ix(&self) -> usize {
        self.active
            .as_deref()
            .and_then(|p| self.tree.visible_index(p))
            .unwrap_or(0)
    }

    fn focus_down(&mut self, _: &FocusDown, _: &mut Window, cx: &mut Context<Self>) {
        let next = match self.focused_ix() {
            Some(ix) => (ix + 1).min(self.tree.len().saturating_sub(1)),
            None => self.start_ix(),
        };
        self.set_focused_ix(next, cx);
    }

    fn focus_up(&mut self, _: &FocusUp, _: &mut Window, cx: &mut Context<Self>) {
        let next = match self.focused_ix() {
            Some(ix) => ix.saturating_sub(1),
            None => self.start_ix(),
        };
        self.set_focused_ix(next, cx);
    }

    /// Right opens a collapsed folder, steps into an open one, and does
    /// nothing on a file.
    fn focus_right(&mut self, _: &FocusRight, _: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.focused_ix() else {
            return;
        };
        let Some(row) = self.tree.row(ix).cloned() else {
            return;
        };
        if !row.is_folder {
            return;
        }
        if self.tree.is_collapsed(&row.path) {
            self.tree.toggle(&row.path);
            cx.notify();
        } else if self.tree.row(ix + 1).is_some_and(|r| r.depth > row.depth) {
            self.set_focused_ix(ix + 1, cx);
        }
    }

    /// Left closes an open folder; on a file or a closed folder it climbs
    /// to the parent and closes that, or does nothing at the root.
    fn focus_left(&mut self, _: &FocusLeft, _: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.focused_ix() else {
            return;
        };
        let Some(row) = self.tree.row(ix).cloned() else {
            return;
        };
        if row.is_folder && !self.tree.is_collapsed(&row.path) {
            self.tree.toggle(&row.path);
            cx.notify();
            return;
        }
        let Some((parent, _)) = row.path.rsplit_once('/') else {
            return;
        };
        let parent = parent.to_string();
        if !self.tree.is_collapsed(&parent) {
            self.tree.toggle(&parent);
        }
        if let Some(parent_ix) = self.tree.visible_index(&parent) {
            self.set_focused_ix(parent_ix, cx);
        }
        cx.notify();
    }

    /// Enter opens the file or toggles the folder under the ring.
    fn activate(&mut self, _: &Activate, _: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.focused_ix().and_then(|ix| self.tree.row(ix).cloned()) else {
            return;
        };
        if row.is_folder {
            self.tree.toggle(&row.path);
            cx.notify();
        } else {
            cx.emit(ExplorerEvent::Open(row.path));
        }
    }

    /// Space toggles a folder and leaves a file alone.
    fn toggle_folder(&mut self, _: &ToggleFolder, _: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.focused_ix().and_then(|ix| self.tree.row(ix).cloned()) else {
            return;
        };
        if row.is_folder {
            self.tree.toggle(&row.path);
            cx.notify();
        }
    }

    fn leave_tree(&mut self, _: &LeaveTree, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(ExplorerEvent::LeaveFocus);
    }

    fn rename_focused(&mut self, _: &RenameFocused, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(row) = self.focused_ix().and_then(|ix| self.tree.row(ix).cloned()) {
            cx.emit(ExplorerEvent::Rename(row.path));
        }
    }

    fn delete_focused(&mut self, _: &DeleteFocused, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(row) = self.focused_ix().and_then(|ix| self.tree.row(ix).cloned()) {
            cx.emit(ExplorerEvent::Delete(row.path));
        }
    }

    /// A rebuilt tree: the folders that were open stay open, the sort order
    /// carries over and the open file keeps its highlight. Nothing is
    /// expanded or scrolled, so a file written elsewhere in the vault does
    /// not move the list.
    pub fn set_tree(&mut self, mut tree: FileTree, cx: &mut Context<Self>) {
        let expanded = self.tree.expanded_folders();
        tree.expand_folders(&expanded);
        tree.set_sort(self.tree.sort());
        self.tree = tree;
        cx.notify();
    }

    /// The leaf's state as `workspace.json` recorded it.
    pub fn set_leaf_state(&mut self, sort: SortOrder, auto_reveal: bool, cx: &mut Context<Self>) {
        self.tree.set_sort(sort);
        self.auto_reveal = auto_reveal;
        cx.notify();
    }

    pub fn sort_order(&self) -> SortOrder {
        self.tree.sort()
    }

    pub fn auto_reveal(&self) -> bool {
        self.auto_reveal
    }

    fn set_sort(&mut self, sort: SortOrder, cx: &mut Context<Self>) {
        self.tree.set_sort(sort);
        self.sort_menu_open = false;
        cx.notify();
    }

    /// "Auto-reveal current file": switching it on reveals the open file
    /// right away, as Obsidian does.
    fn toggle_auto_reveal(&mut self, cx: &mut Context<Self>) {
        self.auto_reveal = !self.auto_reveal;
        if self.auto_reveal {
            if let Some(active) = self.active.clone() {
                self.reveal(&active, cx);
            }
        }
        cx.notify();
    }

    /// "Expand all" opens every folder at every depth; "Collapse all", the
    /// same button once any folder is open, closes them all.
    fn toggle_all(&mut self, cx: &mut Context<Self>) {
        if self.tree.any_expanded() {
            self.tree.collapse_all();
        } else {
            self.tree.expand_all();
        }
        cx.notify();
    }

    /// The file that just became the open one: highlighted, and with
    /// auto-reveal on, also expanded into view.
    pub fn follow(&mut self, path: &str, cx: &mut Context<Self>) {
        if self.auto_reveal {
            self.reveal(path, cx);
        } else {
            self.set_active(path, cx);
        }
    }

    /// Marks `path` as the open file without touching the tree: its row is
    /// highlighted if a folder is not hiding it.
    pub fn set_active(&mut self, path: &str, cx: &mut Context<Self>) {
        if self.active.as_deref() != Some(path) {
            self.active = Some(path.to_string());
            cx.notify();
        }
    }

    /// No file is open: nothing is highlighted.
    pub fn clear_active(&mut self, cx: &mut Context<Self>) {
        if self.active.take().is_some() {
            cx.notify();
        }
    }

    /// Marks `path` as the open file and scrolls it into view.
    pub fn reveal(&mut self, path: &str, cx: &mut Context<Self>) {
        self.active = Some(path.to_string());
        self.show(path, cx);
    }

    /// Scrolls `path` into view without claiming it is the open file.
    ///
    /// A folder is never the open file, so revealing one — a bookmarked
    /// folder, or a folder just created — leaves the highlight on the note
    /// that is open, as Obsidian does.
    pub fn reveal_folder(&mut self, path: &str, cx: &mut Context<Self>) {
        self.show(path, cx);
    }

    fn show(&mut self, path: &str, cx: &mut Context<Self>) {
        if let Some(ix) = self.tree.reveal(path) {
            self.scroll.scroll_to_item(ix, ScrollStrategy::Center);
        }
        cx.notify();
    }

    /// What the tree is showing, for the control socket's state dump: the
    /// rows a collapsed folder hides are not in it, because they are not on
    /// screen either.
    pub fn describe(&self, rows: usize) -> Value {
        let total = self.tree.len();
        let shown: Vec<Value> = self
            .tree
            .rows()
            .take(rows)
            .map(|row| {
                json!({
                    "path": row.path,
                    "name": row.name,
                    "depth": row.depth,
                    "folder": row.is_folder,
                    "kind": kind_name(row.kind),
                })
            })
            .collect();
        json!({
            "active": self.active,
            "focused": self.focused,
            "sort_order": self.tree.sort().as_str(),
            "auto_reveal": self.auto_reveal,
            "any_expanded": self.tree.any_expanded(),
            "rows": shown,
            "rows_total": if total > shown.len() { Some(total) } else { None },
            "menu": self.menu.as_ref().map(Self::describe_menu),
        })
    }

    /// The open context menu as the socket sees it: what it was opened on and
    /// the rows a reader would see, `"---"` per separator. Without this a
    /// harness can right-click but cannot read back what came up, which is
    /// what kept the menus untestable from outside the process.
    fn describe_menu(menu: &Menu) -> Value {
        let (target, path) = match &menu.target {
            Target::File(path) => ("file", path.as_str()),
            Target::Folder(path) => ("folder", path.as_str()),
            Target::Blank => ("blank", ""),
        };
        let mut items: Vec<Value> = Vec::new();
        let mut previous = None;
        for item in menu_items(&menu.target) {
            for _ in 0..separators_above(previous, item.section) {
                items.push(json!("---"));
            }
            previous = Some(item.section);
            items.push(json!(item.label));
        }
        json!({ "target": target, "path": path, "items": items })
    }

    /// A click on a folder toggles it and puts the keyboard on it; a click
    /// on the open file's row puts the keyboard on that row; a click that
    /// opens a file hands the keyboard to the editor, as Obsidian's does.
    fn click(&mut self, ix: usize, new_tab: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.tree.row(ix).cloned() else {
            return;
        };
        if row.is_folder {
            self.tree.toggle(&row.path);
            self.focused = Some(row.path);
            window.focus(&self.focus_handle, cx);
            cx.notify();
        } else if new_tab {
            cx.emit(ExplorerEvent::OpenInNewTab(row.path));
        } else if self.active.as_deref() == Some(row.path.as_str()) {
            self.focused = Some(row.path);
            window.focus(&self.focus_handle, cx);
            cx.notify();
        } else {
            self.focused = Some(row.path.clone());
            cx.emit(ExplorerEvent::Open(row.path));
        }
    }

    fn open_menu(&mut self, ix: usize, position: Point<Pixels>, cx: &mut Context<Self>) {
        if let Some(row) = self.tree.row(ix).cloned() {
            let target = match row.is_folder {
                true => Target::Folder(row.path),
                false => Target::File(row.path),
            };
            self.menu = Some(Menu { position, target });
            cx.notify();
        }
    }

    /// A right-click on the empty space below the last row: Obsidian's
    /// four-item menu, rooted at the vault.
    fn open_blank_menu(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.menu = Some(Menu {
            position,
            target: Target::Blank,
        });
        cx.notify();
    }

    fn close_menu(&mut self, cx: &mut Context<Self>) {
        if self.menu.take().is_some() {
            cx.notify();
        }
    }

    fn render_menu(&mut self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let menu = self.menu.as_ref()?;
        let theme = cx.global::<Theme>().clone();
        let items = menu_items(&menu.target);
        let mut list = div()
            .id("explorer-menu")
            .flex()
            .flex_col()
            .min_w(px(180.))
            .py_1()
            .bg(theme.bg_secondary)
            .border_1()
            .border_color(theme.border)
            .rounded_md()
            .shadow_lg()
            .text_size(px(13.))
            .text_color(theme.text)
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_menu(cx)));
        let mut previous: Option<Section> = None;
        for (ix, item) in items.into_iter().enumerate() {
            for n in 0..separators_above(previous, item.section) {
                list = list.child(
                    div()
                        .id(("explorer-menu-separator", ix * 2 + n))
                        .my_1()
                        .h(px(1.))
                        .w_full()
                        .bg(theme.border),
                );
            }
            previous = Some(item.section);
            let event = std::cell::Cell::new(Some(item.event));
            list = list.child(
                div()
                    .id(("explorer-menu-item", ix))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.bg_alt))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.close_menu(cx);
                        if let Some(event) = event.take() {
                            cx.emit(event);
                        }
                    }))
                    .child(crate::assets::icon(item.icon, 16., theme.text_muted))
                    .child(item.label),
            );
        }
        Some(
            deferred(
                anchored()
                    .position(menu.position)
                    .snap_to_window_with_margin(px(8.))
                    .child(list),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    /// A header button: Obsidian's `nav-action-button`, 28 by 24 with a
    /// 4 px radius, the icon muted until the pointer is over it; `active`
    /// is the wash and accent of an `is-active` one.
    fn header_button(
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

    fn render_header(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let theme = cx.global::<Theme>().clone();
        let any_expanded = self.tree.any_expanded();
        let (fold_id, fold_icon, fold_label) = if any_expanded {
            ("explorer-collapse-all", "chevrons-down-up", "Collapse all")
        } else {
            ("explorer-expand-all", "chevrons-up-down", "Expand all")
        };
        let sort_menu = self.render_sort_menu(&theme, cx);
        div()
            .flex_shrink_0()
            .h(px(HEADER_HEIGHT))
            .p(px(8.))
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .gap(px(2.))
            .child(
                Self::header_button("explorer-new-note", "square-pen", "New note", false, &theme)
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(ExplorerEvent::NewNoteFromHeader))),
            )
            .child(
                Self::header_button(
                    "explorer-new-folder",
                    "folder-plus",
                    "New folder",
                    false,
                    &theme,
                )
                .on_click(cx.listener(|_, _, _, cx| {
                    cx.emit(ExplorerEvent::NewFolder {
                        parent: String::new(),
                    })
                })),
            )
            .child(
                // The menu hangs off this wrapper, so it opens under the
                // button wherever the dock is.
                div()
                    .relative()
                    .child(
                        Self::header_button(
                            "explorer-sort",
                            "arrow-up-narrow-wide",
                            "Change sort order",
                            false,
                            &theme,
                        )
                        .on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.sort_menu_open = !this.sort_menu_open;
                                cx.notify();
                            },
                        )),
                    )
                    .children(sort_menu),
            )
            .child(
                Self::header_button(
                    "explorer-auto-reveal",
                    "gallery-vertical",
                    "Auto-reveal current file",
                    self.auto_reveal,
                    &theme,
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_auto_reveal(cx))),
            )
            .child(
                Self::header_button(fold_id, fold_icon, fold_label, false, &theme)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_all(cx))),
            )
    }

    /// "Change sort order": the six orders in three pairs, a check against
    /// the one in force.
    fn render_sort_menu(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if !self.sort_menu_open {
            return None;
        }
        let current = self.tree.sort();
        let mut list = div()
            .id("explorer-sort-menu")
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
                this.sort_menu_open = false;
                cx.notify();
            }));
        for (ix, order) in SortOrder::ALL.into_iter().enumerate() {
            if ix > 0 && ix % 2 == 0 {
                list = list.child(div().my_1().h(px(1.)).bg(theme.border));
            }
            let checked = order == current;
            list = list.child(
                div()
                    .id(("explorer-sort-item", ix))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.bg_alt))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_sort(order, cx)))
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
                    .offset(point(px(0.), px(26.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(list),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    fn render_rows(
        &mut self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::Stateful<gpui::Div>> {
        let theme = cx.global::<Theme>().clone();
        let focused_ix = self.focused_ix();
        range
            .filter_map(|ix| {
                let row = self.tree.row(ix)?;
                let active = self.active.as_deref() == Some(row.path.as_str());
                let focused = focused_ix == Some(ix);
                let marker = row.is_folder.then(|| {
                    if self.tree.is_collapsed(&row.path) {
                        "chevron-right"
                    } else {
                        "chevron-down"
                    }
                });
                let name: SharedString = row.name.clone().into();
                let tag: Option<SharedString> = row.tag.clone().map(Into::into);
                // A file no view can open stays faint; a canvas or an image
                // reads like a note, its tag saying what it is.
                let faint = !row.is_folder && row.kind == FileKind::Other;
                // Obsidian's row: 13 px text in a 25 px row with an 8 px
                // radius, muted until it is active or under the pointer.
                Some(
                    div()
                        .id(("row", ix))
                        .w_full()
                        .h(px(25.))
                        .mb(px(2.))
                        .flex()
                        .flex_row()
                        .items_center()
                        // The 2 px border is the keyboard's ring (Obsidian's
                        // `has-focus`), transparent on every other row so the
                        // text does not move when the ring arrives.
                        .border_2()
                        .border_color(if focused {
                            theme.border_focus
                        } else {
                            gpui::transparent_black()
                        })
                        .pl(px(6. + row.depth as f32 * 14.))
                        .pr(px(6.))
                        .rounded(px(8.))
                        .cursor_pointer()
                        .text_size(px(13.))
                        .text_color(if faint {
                            theme.text_faint
                        } else if active {
                            theme.text
                        } else {
                            theme.text_muted
                        })
                        .when(active, |d| d.bg(theme.bg_active))
                        .hover(|s| s.bg(theme.bg_alt).text_color(theme.text))
                        .on_click(
                            cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                                let modifiers = event.modifiers();
                                this.click(ix, modifiers.control || modifiers.platform, window, cx)
                            }),
                        )
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                                // The empty space below the rows has its own
                                // right-click; without this the row's menu
                                // would open and then be replaced by that one.
                                cx.stop_propagation();
                                this.open_menu(ix, event.position, cx)
                            }),
                        )
                        .child(
                            div()
                                .w(px(16.))
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .opacity(0.85)
                                .children(
                                    marker.map(|name| {
                                        crate::assets::icon(name, 14., theme.text_muted)
                                    }),
                                ),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(name),
                        )
                        .children(tag.map(|tag| {
                            // Obsidian's `nav-file-tag`: small upper-case
                            // text at the row's right end.
                            div()
                                .flex_shrink_0()
                                .ml_2()
                                .text_size(px(TAG_SIZE))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.text_faint)
                                .child(tag)
                        })),
                )
            })
            .collect()
    }
}

impl Render for FileExplorer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.tree.len();
        let menu = self.render_menu(cx);
        let header = self.render_header(cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .key_context("Explorer")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::focus_up))
            .on_action(cx.listener(Self::focus_down))
            .on_action(cx.listener(Self::focus_left))
            .on_action(cx.listener(Self::focus_right))
            .on_action(cx.listener(Self::activate))
            .on_action(cx.listener(Self::toggle_folder))
            .on_action(cx.listener(Self::leave_tree))
            .on_action(cx.listener(Self::rename_focused))
            .on_action(cx.listener(Self::delete_focused))
            .child(header)
            // The list is wrapped so that the space below the last row
            // takes a right-click of its own: `uniform_list` hands an event
            // outside every row straight through, and a row's own handler
            // stops propagation so only one of the two menus opens.
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .min_h(px(0.))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                            this.open_blank_menu(event.position, cx)
                        }),
                    )
                    .child(
                        uniform_list("file-explorer", count, cx.processor(Self::render_rows))
                            .track_scroll(&self.scroll)
                            .size_full()
                            .px_1(),
                    ),
            )
            .children(menu)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(paths: &[&str]) -> Vec<VaultFile> {
        paths
            .iter()
            .map(|p| VaultFile {
                path: p.to_string(),
                kind: FileKind::from_path(p),
                size: 0,
                mtime: 0,
                ctime: 0,
            })
            .collect()
    }

    /// The orders Obsidian showed on 2026-10-21 for a vault whose files
    /// were created in one order and modified in another (the
    /// `explorer-header` probe): folders stay in name order under every
    /// sort, and only the files of each folder move.
    #[test]
    fn the_six_sort_orders_move_files_but_not_folders() {
        let created = [
            "Alpha.md",
            "beta.md",
            "Gamma.md",
            "n2.md",
            "n10.md",
            "Projects/Plan.md",
            "Projects/Deep/Inner.md",
            "Archive/old.md",
            "Projects/Budget.md",
            "cover.png",
        ];
        let modified = [
            "n10.md",
            "Archive/old.md",
            "Alpha.md",
            "Projects/Budget.md",
            "cover.png",
            "Gamma.md",
            "Projects/Deep/Inner.md",
            "beta.md",
            "Projects/Plan.md",
            "n2.md",
        ];
        let files: Vec<VaultFile> = created
            .iter()
            .enumerate()
            .map(|(ix, p)| VaultFile {
                path: p.to_string(),
                kind: FileKind::from_path(p),
                size: 0,
                mtime: 1 + modified.iter().position(|m| m == p).unwrap() as u64,
                ctime: 1 + ix as u64,
            })
            .collect();
        let empty = vec!["Empty".to_string()];
        let mut tree = FileTree::new(&files, &empty, false);
        tree.expand_all();
        let names = |tree: &FileTree| -> Vec<String> {
            tree.rows()
                .map(|r| {
                    if r.is_folder {
                        format!("{}/", r.name)
                    } else {
                        r.name.clone()
                    }
                })
                .collect()
        };
        let observed: [(SortOrder, &[&str]); 6] = [
            (
                SortOrder::Alphabetical,
                &[
                    "Archive/",
                    "old",
                    "Empty/",
                    "Projects/",
                    "Deep/",
                    "Inner",
                    "Budget",
                    "Plan",
                    "Alpha",
                    "beta",
                    "cover",
                    "Gamma",
                    "n2",
                    "n10",
                ],
            ),
            (
                SortOrder::AlphabeticalReverse,
                &[
                    "Archive/",
                    "old",
                    "Empty/",
                    "Projects/",
                    "Deep/",
                    "Inner",
                    "Plan",
                    "Budget",
                    "n10",
                    "n2",
                    "Gamma",
                    "cover",
                    "beta",
                    "Alpha",
                ],
            ),
            (
                SortOrder::ByModifiedTime,
                &[
                    "Archive/",
                    "old",
                    "Empty/",
                    "Projects/",
                    "Deep/",
                    "Inner",
                    "Plan",
                    "Budget",
                    "n2",
                    "beta",
                    "Gamma",
                    "cover",
                    "Alpha",
                    "n10",
                ],
            ),
            (
                SortOrder::ByModifiedTimeReverse,
                &[
                    "Archive/",
                    "old",
                    "Empty/",
                    "Projects/",
                    "Deep/",
                    "Inner",
                    "Budget",
                    "Plan",
                    "n10",
                    "Alpha",
                    "cover",
                    "Gamma",
                    "beta",
                    "n2",
                ],
            ),
            (
                SortOrder::ByCreatedTime,
                &[
                    "Archive/",
                    "old",
                    "Empty/",
                    "Projects/",
                    "Deep/",
                    "Inner",
                    "Budget",
                    "Plan",
                    "cover",
                    "n10",
                    "n2",
                    "Gamma",
                    "beta",
                    "Alpha",
                ],
            ),
            (
                SortOrder::ByCreatedTimeReverse,
                &[
                    "Archive/",
                    "old",
                    "Empty/",
                    "Projects/",
                    "Deep/",
                    "Inner",
                    "Plan",
                    "Budget",
                    "Alpha",
                    "beta",
                    "Gamma",
                    "n2",
                    "n10",
                    "cover",
                ],
            ),
        ];
        for (order, rows) in observed {
            tree.set_sort(order);
            assert_eq!(names(&tree), rows, "{}", order.as_str());
            assert_eq!(SortOrder::parse(order.as_str()), Some(order));
        }
        // A collapsed folder stays collapsed across a re-sort.
        tree.toggle("Projects");
        tree.set_sort(SortOrder::Alphabetical);
        assert!(tree.is_collapsed("Projects"));
        assert!(tree.any_expanded(), "Archive is still open");
        tree.collapse_all();
        assert!(!tree.any_expanded());
    }

    #[test]
    fn folders_first_then_files_case_insensitively() {
        let files = files(&[
            "zeta.md",
            "Alpha.md",
            "b/inner.md",
            "a/x.png",
            "a/deep/y.md",
            "notes.zip",
        ]);
        let tree = FileTree::new(&files, &[], false);
        let names: Vec<(usize, &str)> = tree.rows().map(|r| (r.depth, r.name.as_str())).collect();
        assert_eq!(names, vec![(0, "a"), (0, "b"), (0, "Alpha"), (0, "zeta")]);
        let empty = vec!["Empty".to_string(), "a/void".to_string()];
        let with_empty = FileTree::new(&files, &empty, false);
        let names: Vec<&str> = with_empty.rows().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "Empty", "Alpha", "zeta"]);
        let shown = FileTree::new(&files, &[], true);
        assert_eq!(
            shown.len(),
            5,
            "notes.zip appears with unsupported files on"
        );
    }

    /// Obsidian collates names numerically, so `n2` comes before `n10`; the
    /// search pane already does and the two panes have to agree.
    #[test]
    fn names_sort_numerically_within_each_group() {
        let files = files(&[
            "n10.md",
            "n2.md",
            "n1.md",
            "n20.md",
            "n3.md",
            "f10/a.md",
            "f2/a.md",
            "Img 10.png",
            "Img 2.png",
        ]);
        let tree = FileTree::new(&files, &[], false);
        let names: Vec<&str> = tree.rows().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["f2", "f10", "Img 2", "Img 10", "n1", "n2", "n3", "n10", "n20"],
            "folders stay ahead of files, and both groups count"
        );
    }

    #[test]
    fn toggle_and_reveal_expand_ancestors() {
        let files = files(&["a/deep/y.md", "a/x.png", "b/inner.md"]);
        let mut tree = FileTree::new(&files, &[], false);
        assert_eq!(tree.len(), 2);
        tree.toggle("a");
        let names: Vec<&str> = tree.rows().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["a", "deep", "x", "b"]);
        tree.toggle("a");
        assert_eq!(tree.reveal("a/deep/y.md"), Some(2));
        let names: Vec<&str> = tree.rows().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["a", "deep", "y", "x", "b"]);
        assert!(tree.rows().nth(2).unwrap().path == "a/deep/y.md");
        tree.collapse_all();
        assert_eq!(tree.len(), 2);
        tree.expand_all();
        assert_eq!(tree.len(), 6);
        let expanded = tree.expanded_folders();
        let mut rebuilt = FileTree::new(&files, &[], false);
        rebuilt.expand_folders(&expanded);
        assert_eq!(rebuilt.len(), 6);
    }
}

#[cfg(test)]
mod menu_tests {
    use super::*;
    use gpui::AssetSource;

    /// The path an event acts on. The match is exhaustive on purpose: a new
    /// [`ExplorerEvent`] will not compile until it says which row it carries,
    /// which is what keeps a menu from ever acting on the wrong one.
    fn payload(event: &ExplorerEvent) -> Option<&str> {
        match event {
            ExplorerEvent::Open(p)
            | ExplorerEvent::OpenInNewTab(p)
            | ExplorerEvent::Rename(p)
            | ExplorerEvent::Move(p)
            | ExplorerEvent::Duplicate(p)
            | ExplorerEvent::Delete(p)
            | ExplorerEvent::Bookmark(p)
            | ExplorerEvent::OpenToRight(p)
            | ExplorerEvent::CopyPath(p)
            | ExplorerEvent::OpenInDefaultApp(p)
            | ExplorerEvent::SearchInFolder(p) => Some(p),
            ExplorerEvent::NewNote { folder }
            | ExplorerEvent::NewCanvas { folder }
            | ExplorerEvent::NewBase { folder } => Some(folder),
            ExplorerEvent::NewFolder { parent } => Some(parent),
            ExplorerEvent::NewNoteFromHeader | ExplorerEvent::LeaveFocus => None,
        }
    }

    /// The menu as a reader sees it: a `---` per separator, then each item's
    /// label. Comparing this against the spec's own table is the point.
    fn drawn(target: &Target) -> Vec<String> {
        let mut out = Vec::new();
        let mut previous = None;
        for item in menu_items(target) {
            for _ in 0..separators_above(previous, item.section) {
                out.push("---".to_string());
            }
            previous = Some(item.section);
            out.push(item.label.to_string());
        }
        out
    }

    /// [[Specification#The file explorer's context menus]], the file-row
    /// table, minus the four items Silica has no command for -- Open in new
    /// window, Merge entire file with..., Open version history and Show in
    /// system explorer. Dropping Open version history empties the `view`
    /// section, so the separator the capture has around it is gone too; every
    /// other separator is where Obsidian put it, including the pair above
    /// Rename.
    #[test]
    fn the_file_rows_menu_is_obsidians_order_without_the_rows_it_cannot_run() {
        assert_eq!(
            drawn(&Target::File("Notes/Plan.md".into())),
            [
                "Open in new tab",
                "Open to the right",
                "---",
                "Make a copy",
                "Move file to...",
                "Bookmark...",
                "---",
                "Copy path",
                "---",
                "Open in default app",
                "---",
                "---",
                "Rename...",
                "Delete",
            ]
        );
    }

    /// The folder-row table, minus "Make a copy" (no recursive folder copy)
    /// and "Show in system explorer". A folder gets no "Open in default app":
    /// the capture has that item on the file row only.
    #[test]
    fn the_folder_rows_menu_is_obsidians_order() {
        assert_eq!(
            drawn(&Target::Folder("Notes".into())),
            [
                "New note",
                "New folder",
                "New canvas",
                "New base",
                "---",
                "Move folder to...",
                "Search in folder",
                "Bookmark...",
                "---",
                "Copy path",
                "---",
                "---",
                "Rename...",
                "Delete",
            ]
        );
    }

    /// "The empty space below the last row gives only the four
    /// `action-primary` items" -- so no separators at all, and every one of
    /// them rooted at the vault.
    #[test]
    fn the_empty_space_offers_the_four_creation_items_at_the_root() {
        assert_eq!(
            drawn(&Target::Blank),
            ["New note", "New folder", "New canvas", "New base"]
        );
        for item in menu_items(&Target::Blank) {
            assert_eq!(
                payload(&item.event),
                Some(""),
                "{} was not rooted at the vault",
                item.label
            );
        }
    }

    /// Obsidian's file row has no creation items. Silica's menu used to open
    /// with "New note" and "New folder" on a file, which is the one thing in
    /// the captured menus that was wrong rather than merely missing.
    #[test]
    fn a_file_row_offers_nothing_that_creates_a_file() {
        for item in menu_items(&Target::File("Plan.md".into())) {
            assert!(
                !item.label.starts_with("New "),
                "a file row offered {}",
                item.label
            );
        }
    }

    /// Every item carries the folder or file it was opened on, so a menu can
    /// never act on the wrong row.
    #[test]
    fn every_item_carries_the_row_it_was_opened_on() {
        for (target, want) in [
            (Target::Folder("Notes/Deep".into()), "Notes/Deep"),
            (
                Target::File("Notes/Deep/Plan.md".into()),
                "Notes/Deep/Plan.md",
            ),
        ] {
            for item in menu_items(&target) {
                assert_eq!(
                    payload(&item.event),
                    Some(want),
                    "{} acts on the wrong row",
                    item.label
                );
            }
        }
    }

    /// A separator where the section changes, two above `danger`, none at the
    /// top, and none between two items of one section.
    #[test]
    fn a_separator_falls_where_the_section_changes() {
        assert_eq!(separators_above(None, Section::Open), 0);
        assert_eq!(separators_above(Some(Section::Open), Section::Open), 0);
        assert_eq!(separators_above(Some(Section::Open), Section::Action), 1);
        assert_eq!(
            separators_above(Some(Section::Action), Section::Unsectioned),
            1
        );
        assert_eq!(separators_above(Some(Section::System), Section::Danger), 2);
        assert_eq!(separators_above(Some(Section::Danger), Section::Danger), 0);
    }

    /// The icon of every item in every menu is bundled and is the name
    /// Obsidian leaves in its DOM, or Lucide's current spelling of it where
    /// Obsidian's is an old alias: `lucide-edit` is `square-pen` and
    /// `lucide-edit-3` is `pen-line`, both from Lucide's own alias data.
    #[test]
    fn every_menu_icon_is_bundled() {
        let targets = [
            Target::File("Plan.md".into()),
            Target::Folder("Notes".into()),
            Target::Blank,
        ];
        for target in targets {
            for item in menu_items(&target) {
                let path = format!("icons/{}.svg", item.icon);
                assert!(
                    crate::assets::Assets.load(&path).unwrap().is_some(),
                    "{} wants {path}, which is not bundled",
                    item.label
                );
            }
        }
    }
}
