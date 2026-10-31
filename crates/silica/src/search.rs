//! The Search view (Ctrl+Shift+F): Obsidian's `search-row` (the query
//! field with its Match case decorator and clear button, then the Search
//! settings button), the "Explain search terms" block, the three settings
//! toggles, the `search-results-info` bar (result count with its menu, the
//! sort order) and the result list — a title row per file over a card of
//! its matches, each match wrapping with the matched text highlighted.
//!
//! Each change to the query runs the compiled search over every indexed
//! note on the background executor; a generation counter drops results of
//! superseded runs. Changing the sort order re-sorts the list it has, the
//! way Obsidian's `setSortOrder` does, without running the query again.
//!
//! The six keys Obsidian serialises for the leaf (`query`, `matchingCase`,
//! `explainSearch`, `collapseAll`, `extraContext`, `sortOrder`) live here;
//! the workspace reads and writes them through [`SearchPane::restore`] and
//! [`SearchPane::leaf_state`].

use crate::assets::icon;
use crate::canvas::menu::tooltip;
use crate::explorer::SortOrder;
use crate::text_input::{InputEvent, TextInput};
use crate::theme::{Theme, FONT_UI_SMALL, FONT_UI_SMALLER, ICON_SIZE};
use crate::workspace::{Changes, VaultState};
use gpui::{
    anchored, deferred, div, list, prelude::*, px, relative, rgb, Anchor, App, ClickEvent, Context,
    Entity, EventEmitter, FocusHandle, Focusable, HighlightStyle, ListAlignment, ListState, Pixels,
    Point, SharedString, StyledText, Subscription, Window,
};
use rayon::prelude::*;
use serde_json::{json, Value};
use silica_cache::{all_tags, CachedMetadata};
use silica_search::{
    context_window, match_lines, natural_cmp, parse, MatchLine, Query, Search, SearchFile, TaskLine,
};
use silica_vault::FileChange;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct FileResult {
    pub path: String,
    pub name: String,
    /// Hit ranges inside `name`, when the query matched the file name.
    pub name_ranges: Vec<Range<usize>>,
    pub matches: Vec<MatchLine>,
    /// Every hit range in the note's text, for the context a row grows
    /// into when it is asked for more.
    pub hits: Vec<Range<usize>>,
    /// The file's times, for the four time orders.
    pub mtime: u64,
    pub ctime: u64,
}

impl FileResult {
    /// What this file contributes to the result count: one per match line,
    /// plus one for a name hit, which Obsidian counts but leaves out of the
    /// number beside the file name.
    fn results(&self) -> usize {
        self.matches.len() + usize::from(!self.name_ranges.is_empty())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    File { ix: usize },
    Match { ix: usize, m: usize },
}

impl Row {
    fn file(self) -> usize {
        match self {
            Row::File { ix } | Row::Match { ix, .. } => ix,
        }
    }
}

/// How a row's text is highlighted. `Name` is a file title's (13 px text
/// colour kept), `Match` a context line's; both carry only the background,
/// as Obsidian's `search-result-file-matched-text` does.
#[derive(Clone, Copy)]
enum Highlight {
    Name,
    Match,
}

pub enum SearchEvent {
    /// Open a file at a byte offset.
    Open(String, Option<usize>),
    /// "Bookmark..." from the count's menu: bookmark this query.
    Bookmark(String),
    /// "Copy search results" from the count's menu: open the copy modal
    /// over the listed files, in list order.
    CopyResults(Vec<CopyEntry>),
    /// Escape with nothing left to clear: the keyboard goes back to the
    /// main view.
    Dismiss,
    /// One of the six serialised keys changed; the layout wants saving.
    StateChanged,
    /// Vault files were modified by a replace operation.
    FilesChanged(Changes),
}

/// One line of the "Copy search results" modal: a file in the result list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyEntry {
    pub path: String,
    pub name: String,
}

/// `.search-result-container` padding, and the row metrics inside it.
const CONTAINER_PAD: f32 = 12.;
const FILE_ROW_HEIGHT: f32 = 25.;
/// The accent at a third: Obsidian's match-row hover wash.
const HOVER_ALPHA: f32 = 0.33;
/// The accent at a tenth: the wash behind an active header button.
const ACTIVE_ALPHA: f32 = 0.1;

pub struct SearchPane {
    input: Entity<TextInput>,
    replace_input: Entity<TextInput>,
    replace_open: bool,
    state: Option<Arc<VaultState>>,
    results: Vec<FileResult>,
    /// `matchingCase`: the Match case decorator in the field.
    matching_case: bool,
    /// `explainSearch`: the block above the settings describing the query.
    explain: bool,
    /// `collapseAll`: every file row starts folded.
    collapse_all: bool,
    /// `extraContext`: a match shows the lines either side of its own.
    extra_context: bool,
    /// `sortOrder`.
    sort: SortOrder,
    /// Whether the settings section is shown (not serialised).
    settings_open: bool,
    /// A non-empty query has run this session: an emptied one then reads
    /// "0 results" rather than nothing, as Obsidian's count does.
    has_run: bool,
    /// Files whose fold differs from `collapse_all`. Keyed by path so a
    /// re-run over a changed vault leaves the user's folds alone.
    toggled: HashSet<String>,
    /// Lines of context a match grew by its chevrons: (before, after),
    /// keyed by file and line offset.
    expanded: HashMap<(String, usize), (usize, usize)>,
    rows: Vec<Row>,
    /// The row the keyboard is on (Obsidian's `has-focus`).
    focused: Option<usize>,
    list: ListState,
    error: Option<SharedString>,
    generation: u64,
    count_menu_open: bool,
    sort_menu_open: bool,
    menu_at: Point<Pixels>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SearchEvent> for SearchPane {}

impl Focusable for SearchPane {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl SearchPane {
    pub fn new(cx: &mut Context<Self>) -> SearchPane {
        let input = cx.new(|cx| {
            let mut input = TextInput::new("Search...", cx);
            // Obsidian's `search-input-container`: the text starts 30 px
            // in, behind the magnifier.
            input.set_pad_left(px(30.));
            input
        });
        let replace_input = cx.new(|cx| {
            let mut input = TextInput::new("Replace...", cx);
            input.set_pad_left(px(30.));
            input
        });
        let mut subscriptions = vec![cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
            match event {
                InputEvent::Changed => {
                    // A new query builds a new result list, the way
                    // Obsidian's startSearch empties its own; only a re-run
                    // over the same query keeps the folds.
                    this.toggled.clear();
                    this.expanded.clear();
                    this.focused = None;
                    this.run(cx);
                    cx.emit(SearchEvent::StateChanged);
                }
                InputEvent::Down => this.focus_step(1, cx),
                InputEvent::Up => this.focus_step(-1, cx),
                InputEvent::Submit => this.activate_focused(cx),
                InputEvent::Cancel => {
                    if this.focused.is_some() {
                        this.focused = None;
                        cx.notify();
                    } else {
                        cx.emit(SearchEvent::Dismiss);
                    }
                }
            }
        })];
        subscriptions.push(cx.subscribe(&replace_input, |this, _, event: &InputEvent, cx| {
            match event {
                InputEvent::Submit => this.replace_all(cx),
                InputEvent::Cancel => {
                    this.replace_open = false;
                    cx.notify();
                }
                _ => {}
            }
        }));
        SearchPane {
            input,
            replace_input,
            replace_open: false,
            state: None,
            results: Vec::new(),
            matching_case: false,
            explain: false,
            collapse_all: false,
            extra_context: false,
            sort: SortOrder::default(),
            settings_open: false,
            has_run: false,
            toggled: HashSet::new(),
            expanded: HashMap::new(),
            rows: Vec::new(),
            focused: None,
            list: ListState::new(0, ListAlignment::Top, px(400.)),
            error: None,
            generation: 0,
            count_menu_open: false,
            sort_menu_open: false,
            menu_at: Point::default(),
            _subscriptions: subscriptions,
        }
    }

    pub fn set_state(&mut self, state: Arc<VaultState>, cx: &mut Context<Self>) {
        self.state = Some(state);
        self.run(cx);
    }

    /// Sets the field's text. The same text again is left alone: the field
    /// would report a change, and a change empties the folds and the focus
    /// and runs the query afresh, which a second Ctrl+Shift+F must not do.
    pub fn set_query(&mut self, query: &str, cx: &mut Context<Self>) {
        if self.input.read(cx).text() == query {
            return;
        }
        self.input.update(cx, |i, cx| i.set_text(query, cx));
    }

    pub fn query(&self, cx: &App) -> String {
        self.input.read(cx).text().to_string()
    }

    /// Runs the current query again, after the vault changed.
    pub fn rerun(&mut self, cx: &mut Context<Self>) {
        self.run(cx);
    }

    #[allow(dead_code)]
    pub fn replace_focus_handle(&self, cx: &App) -> FocusHandle {
        self.replace_input.focus_handle(cx)
    }

    #[allow(dead_code)]
    pub fn set_replace_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.replace_open != open {
            self.replace_open = open;
            cx.notify();
        }
    }

    #[allow(dead_code)]
    pub fn replace_open(&self) -> bool {
        self.replace_open
    }

    #[allow(dead_code)]
    pub fn set_replace_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.replace_input.update(cx, |i, cx| i.set_text(text, cx));
    }

    #[allow(dead_code)]
    pub fn open_replace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.replace_open = true;
        window.focus(&self.replace_input.focus_handle(cx), cx);
        cx.notify();
    }

    pub fn replace_all(&mut self, cx: &mut Context<Self>) {
        self.replace_paths(None, cx);
    }

    pub fn replace_file(&mut self, path: &str, cx: &mut Context<Self>) {
        self.replace_paths(Some(path), cx);
    }

    fn replace_paths(&mut self, target_path: Option<&str>, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let query = self.input.read(cx).text().trim().to_string();
        if query.is_empty() {
            return;
        }
        let replacement = self.replace_input.read(cx).text().to_string();
        let re = regex_for_query(&query, self.matching_case);

        let mut modified = Vec::new();
        for file in &self.results {
            if let Some(target) = target_path {
                if file.path != target {
                    continue;
                }
            }
            if file.hits.is_empty() {
                continue;
            }
            let orig_text = match state.texts().get(&file.path) {
                Some(t) => t.to_string(),
                None => match std::fs::read_to_string(state.vault().absolute(&file.path)) {
                    Ok(t) => t,
                    Err(_) => continue,
                },
            };
            let new_text = apply_replacements(&orig_text, &file.hits, re.as_ref(), &replacement);
            if new_text != orig_text {
                if state.vault().write_bytes(&file.path, new_text.as_bytes()).is_ok() {
                    modified.push(FileChange::Modified(file.path.clone()));
                }
            }
        }

        if !modified.is_empty() {
            let changes = state.absorb(modified);
            cx.emit(SearchEvent::FilesChanged(changes));
        }
    }

    // ----- leaf state ---------------------------------------------------------

    /// The leaf's `state` from `workspace.json`: the six keys Obsidian
    /// writes, each at its default when missing or unreadable. The query
    /// is set last so the search it starts sees the other five.
    pub fn restore(&mut self, state: &Value, cx: &mut Context<Self>) {
        let flag = |key: &str| state.get(key).and_then(Value::as_bool).unwrap_or(false);
        self.matching_case = flag("matchingCase");
        self.explain = flag("explainSearch");
        self.collapse_all = flag("collapseAll");
        self.extra_context = flag("extraContext");
        self.sort = state
            .get("sortOrder")
            .and_then(Value::as_str)
            .and_then(SortOrder::parse)
            .unwrap_or_default();
        let query = state
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        self.set_query(&query, cx);
        cx.notify();
    }

    /// The six keys, in the order Obsidian emits them.
    pub fn leaf_state(&self, cx: &App) -> Vec<(&'static str, Value)> {
        vec![
            ("query", json!(self.input.read(cx).text())),
            ("matchingCase", json!(self.matching_case)),
            ("explainSearch", json!(self.explain)),
            ("collapseAll", json!(self.collapse_all)),
            ("extraContext", json!(self.extra_context)),
            ("sortOrder", json!(self.sort.as_str())),
        ]
    }

    /// What the pane is showing, for the control socket's state dump: the
    /// count of the summary line and the files behind it, the message a bad
    /// query produced, the settings, the focused row and the result rows
    /// themselves. A collapsed file still reports its matches, since the
    /// count beside its name does too.
    pub fn describe(&self, rows: usize, cx: &App) -> Value {
        let total: usize = self.results.iter().map(FileResult::results).sum();
        // The note text turns a match's byte offset into a line number; it is
        // absent for a file the vault dropped since the search ran.
        let texts = self.state.as_ref().map(|state| state.texts());
        let results: Vec<Value> = self
            .results
            .iter()
            .take(rows)
            .map(|file| {
                let text = texts.as_ref().and_then(|t| t.get(&file.path));
                let hits = file.matches.len();
                let matches: Vec<Value> = file
                    .matches
                    .iter()
                    .take(rows)
                    .map(|m| {
                        json!({
                            "line": text.and_then(|t| line_number(t.as_str(), m.offset)),
                            "text": m.snippet,
                        })
                    })
                    .collect();
                // One note can hold more hits than the dump carries, so it
                // says how many it has, the same way every other capped list
                // in the dump does: `null` when nothing was dropped.
                let shown = matches.len();
                json!({
                    "path": file.path,
                    "name": file.name,
                    "name_match": !file.name_ranges.is_empty(),
                    "collapsed": self.is_collapsed(&file.path),
                    "matches": matches,
                    "matches_total": if hits > shown { Some(hits) } else { None },
                })
            })
            .collect();
        let focused = self
            .focused
            .and_then(|ix| self.rows.get(ix))
            .map(|row| match row {
                Row::File { ix } => json!({"path": self.results[*ix].path, "match": Value::Null}),
                Row::Match { ix, m } => json!({"path": self.results[*ix].path, "match": m}),
            });
        json!({
            "query": self.input.read(cx).text(),
            "error": self.error,
            "files": self.results.len(),
            "matches": total,
            "count": self.count_text(cx),
            "matching_case": self.matching_case,
            "explain": self.explain,
            "collapse_all": self.collapse_all,
            "extra_context": self.extra_context,
            "sort_order": self.sort.as_str(),
            "settings_open": self.settings_open,
            "focused": focused,
            "scroll_top": self.list.logical_scroll_top().item_ix,
            "results": results,
        })
    }

    // ----- running ------------------------------------------------------------

    fn run(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).text().trim().to_string();
        self.generation += 1;
        let generation = self.generation;
        let Some(state) = self.state.clone() else {
            return;
        };
        if query.is_empty() {
            self.results.clear();
            self.error = None;
            self.replace_rows();
            cx.notify();
            return;
        }
        self.has_run = true;
        let tree = parse(&query);
        let tree = if self.matching_case {
            Query::CaseSensitive(Box::new(tree))
        } else {
            tree
        };
        let search = match Search::from_query(tree) {
            Ok(s) => s,
            Err(e) => {
                self.error = Some(e.to_string().into());
                self.results.clear();
                self.replace_rows();
                cx.notify();
                return;
            }
        };
        if search.is_empty() {
            // `(`, `"`, a bare `OR`: the parser read what it could and found
            // no term. Obsidian's startSearch returns before adding a result
            // rather than listing the vault.
            self.results.clear();
            self.error = None;
            self.replace_rows();
            cx.notify();
            return;
        }
        self.error = None;
        let order = self.sort;
        cx.spawn(async move |this, cx| {
            let results = cx
                .background_executor()
                .spawn(async move {
                    let mut results = run_search(&search, &state);
                    sort_results(&mut results, order);
                    results
                })
                .await;
            this.update(cx, |pane, cx| {
                if pane.generation != generation {
                    return;
                }
                pane.results = results;
                pane.replace_rows();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn is_collapsed(&self, path: &str) -> bool {
        self.collapse_all ^ self.toggled.contains(path)
    }

    fn rebuild_rows(&mut self) {
        // A file the new results dropped forgets its fold and its context.
        let live: HashSet<&str> = self.results.iter().map(|f| f.path.as_str()).collect();
        self.toggled.retain(|p| live.contains(p.as_str()));
        self.expanded.retain(|(p, _), _| live.contains(p.as_str()));
        let old = self.rows.len();
        self.rows.clear();
        for (ix, file) in self.results.iter().enumerate() {
            self.rows.push(Row::File { ix });
            if !self.is_collapsed(&file.path) {
                for m in 0..file.matches.len() {
                    self.rows.push(Row::Match { ix, m });
                }
            }
        }
        if self.rows.is_empty() {
            self.focused = None;
        } else if let Some(f) = self.focused.as_mut() {
            *f = (*f).min(self.rows.len() - 1);
        }
        // Replacing the items keeps the scroll position where a reset would
        // not; every row is measured again at the next paint.
        self.list.splice(0..old, self.rows.len());
    }

    /// A new result list: rows rebuilt and the list back at its top.
    fn replace_rows(&mut self) {
        self.rebuild_rows();
        self.list.reset(self.rows.len());
    }

    // ----- folds, focus, settings ---------------------------------------------

    fn toggle(&mut self, path: String, cx: &mut Context<Self>) {
        if !self.toggled.remove(&path) {
            self.toggled.insert(path);
        }
        self.rebuild_rows();
        cx.notify();
    }

    /// Folds or unfolds the focused row's file: Left folds, Right unfolds,
    /// as Obsidian's `onKeyArrowLeftInFocus` and `Right` do. True when the
    /// key did something.
    fn fold_focused(&mut self, collapse: bool, cx: &mut Context<Self>) -> bool {
        let Some(row) = self.focused.and_then(|ix| self.rows.get(ix).copied()) else {
            return false;
        };
        let path = self.results[row.file()].path.clone();
        if self.is_collapsed(&path) == collapse {
            return true;
        }
        // Focus follows the file row, which is the one left on screen.
        if collapse {
            self.focused = self
                .rows
                .iter()
                .position(|r| *r == Row::File { ix: row.file() });
        }
        self.toggle(path, cx);
        true
    }

    fn focus_step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        let next = match self.focused {
            None if delta > 0 => Some(0),
            None => None,
            Some(ix) => {
                let n = ix as isize + delta;
                if n < 0 {
                    None
                } else {
                    Some((n as usize).min(self.rows.len() - 1))
                }
            }
        };
        self.focused = next;
        if let Some(ix) = next {
            self.list.scroll_to_reveal_item(ix);
        }
        cx.notify();
    }

    /// Enter on a focused row: the file, at the match when it is one, as
    /// `onKeyEnterInFocus` calls `onResultClick`.
    fn activate_focused(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self.focused.and_then(|ix| self.rows.get(ix).copied()) else {
            return;
        };
        let file = &self.results[row.file()];
        let offset = match row {
            Row::File { .. } => None,
            Row::Match { m, .. } => Some(file.matches[m].offset),
        };
        cx.emit(SearchEvent::Open(file.path.clone(), offset));
    }

    fn set_matching_case(&mut self, on: bool, cx: &mut Context<Self>) {
        self.matching_case = on;
        self.run(cx);
        cx.emit(SearchEvent::StateChanged);
        cx.notify();
    }

    fn set_collapse_all(&mut self, on: bool, cx: &mut Context<Self>) {
        self.collapse_all = on;
        self.toggled.clear();
        self.rebuild_rows();
        cx.emit(SearchEvent::StateChanged);
        cx.notify();
    }

    fn set_extra_context(&mut self, on: bool, cx: &mut Context<Self>) {
        self.extra_context = on;
        // Every match's height changes: measure them again.
        self.list.splice(0..self.rows.len(), self.rows.len());
        cx.emit(SearchEvent::StateChanged);
        cx.notify();
    }

    fn set_explain(&mut self, on: bool, cx: &mut Context<Self>) {
        self.explain = on;
        cx.emit(SearchEvent::StateChanged);
        cx.notify();
    }

    /// `setSortOrder`: the list it has, in the new order; the query does
    /// not run again, and the folds and the focused file stay.
    fn set_sort(&mut self, order: SortOrder, cx: &mut Context<Self>) {
        self.sort_menu_open = false;
        if order == self.sort {
            cx.notify();
            return;
        }
        self.sort = order;
        let focused_path = self
            .focused
            .and_then(|ix| self.rows.get(ix))
            .map(|r| self.results[r.file()].path.clone());
        sort_results(&mut self.results, order);
        self.rebuild_rows();
        if let Some(path) = focused_path {
            self.focused = self
                .rows
                .iter()
                .position(|r| matches!(r, Row::File { ix } if self.results[*ix].path == path));
        }
        cx.emit(SearchEvent::StateChanged);
        cx.notify();
    }

    /// One more line of context above or below a match (its chevrons).
    fn grow_context(&mut self, path: String, offset: usize, above: bool, cx: &mut Context<Self>) {
        let entry = self.expanded.entry((path, offset)).or_insert((0, 0));
        if above {
            entry.0 += 1;
        } else {
            entry.1 += 1;
        }
        self.list.splice(0..self.rows.len(), self.rows.len());
        cx.notify();
    }

    /// Left and Right fold and unfold the focused file before the field
    /// moves its caret; with no row focused they stay the field's. gpui
    /// dispatches a key's action before any key listener sees the key, so
    /// this takes the field's own actions in the capture phase.
    fn capture_left(
        &mut self,
        _: &crate::text_input::Left,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.fold_focused(true, cx) {
            cx.stop_propagation();
        }
    }

    fn capture_right(
        &mut self,
        _: &crate::text_input::Right,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.fold_focused(false, cx) {
            cx.stop_propagation();
        }
    }

    // ----- text ---------------------------------------------------------------

    /// The text of the count control: Obsidian's `labelResultCount`, "0
    /// results" once a query has run this session, nothing before.
    fn count_text(&self, cx: &App) -> SharedString {
        if let Some(err) = &self.error {
            return err.clone();
        }
        let total: usize = self.results.iter().map(FileResult::results).sum();
        if self.input.read(cx).text().trim().is_empty() && !self.has_run {
            return SharedString::default();
        }
        // The count names no file count; a file name hit counts as one.
        format!("{total} results").into()
    }

    /// The files in list order, for "Copy search results".
    fn copy_entries(&self) -> Vec<CopyEntry> {
        self.results
            .iter()
            .map(|f| CopyEntry {
                path: f.path.clone(),
                name: f.name.clone(),
            })
            .collect()
    }

    /// What a match row shows: the windowed line, or whole lines with
    /// context once "Show more context" is on or its chevrons were pressed.
    fn match_text(&self, file: &FileResult, m: &MatchLine) -> MatchLine {
        let grown = self
            .expanded
            .get(&(file.path.clone(), m.offset))
            .copied()
            .unwrap_or((0, 0));
        let extra = usize::from(self.extra_context);
        let (before, after) = (extra + grown.0, extra + grown.1);
        if before == 0 && after == 0 {
            return m.clone();
        }
        let Some(texts) = self.state.as_ref().map(|s| s.texts()) else {
            return m.clone();
        };
        let Some(text) = texts.get(&file.path) else {
            return m.clone();
        };
        context_window(text, m.offset, before, after, &file.hits)
    }

    // ----- rendering ----------------------------------------------------------

    fn render_row(
        &mut self,
        row_ix: usize,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = cx.global::<Theme>().clone();
        let Some(row) = self.rows.get(row_ix).copied() else {
            return div().into_any_element();
        };
        let focused = self.focused == Some(row_ix);
        match row {
            Row::File { ix } => self.render_file_row(ix, row_ix, focused, &theme, cx),
            Row::Match { ix, m } => self.render_match_row(ix, m, row_ix, focused, &theme, cx),
        }
    }

    /// Obsidian's `tree-item-self search-result-file-title`: 13 px text in
    /// a 25 px row with an 8 px radius, padded 24 px at the left where the
    /// collapse chevron sits, the match count as a faint flair at the right.
    fn render_file_row(
        &self,
        ix: usize,
        row_ix: usize,
        focused: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let file = &self.results[ix];
        let collapsed = self.is_collapsed(&file.path);
        let path = file.path.clone();
        let name = StyledText::new(SharedString::from(file.name.clone()))
            .with_highlights(highlights(&file.name_ranges, Highlight::Name, theme));
        let count: SharedString = file.matches.len().to_string().into();
        let replace_btn = (self.replace_open && !file.hits.is_empty()).then(|| {
            let file_path = file.path.clone();
            div()
                .id(("search-file-replace", row_ix))
                .w(px(20.))
                .h(px(20.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .cursor_pointer()
                .hover(|s| s.bg(theme.bg_alt))
                .tooltip(tooltip("Replace in this file"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.replace_file(&file_path, cx);
                }))
                .child(icon("arrow-left-right", 12., theme.text_muted))
        });
        div()
            .id(("search-file", row_ix))
            .relative()
            .w_full()
            .h(px(FILE_ROW_HEIGHT))
            .mb(px(2.))
            .pl(px(24.))
            .pr(px(8.))
            .flex()
            .flex_row()
            .items_center()
            .rounded(px(8.))
            .cursor_pointer()
            .hover(|s| s.bg(theme.bg_alt))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.focused = Some(row_ix);
                this.toggle(path.clone(), cx)
            }))
            .child(
                // `tree-item-icon collapse-icon`: 16 px wide in the row's
                // left gutter, the 10 px chevron faint at 85 %.
                div()
                    .absolute()
                    .left(px(4.))
                    .top(px(0.))
                    .w(px(16.))
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .opacity(0.85)
                    .child(icon(
                        if collapsed {
                            "chevron-right"
                        } else {
                            "chevron-down"
                        },
                        10.,
                        theme.text_faint,
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(FONT_UI_SMALL))
                    .text_color(theme.text)
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(name),
            )
            .children(replace_btn)
            .child(
                div()
                    .flex_shrink_0()
                    .pl(px(8.))
                    .text_size(px(FONT_UI_SMALLER))
                    .text_color(theme.text_faint)
                    .child(count),
            )
            .children(focused.then(|| focus_ring(8., theme)))
            .into_any_element()
    }

    /// One `search-result-file-match` in its file's card
    /// (`search-result-file-matches`: the note's background, a 4 px radius,
    /// 4 px above and 8 px below). The rows are separate list items, so
    /// each paints its share of the card: the first its top corners, the
    /// last its bottom ones and the margin under it, and every row after
    /// the first the hairline above it. Under the pointer the row takes
    /// the accent wash and shows its "Show more context" chevrons.
    fn render_match_row(
        &self,
        ix: usize,
        m: usize,
        row_ix: usize,
        focused: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let file = &self.results[ix];
        let line = &file.matches[m];
        let shown = self.match_text(file, line);
        let path = file.path.clone();
        let offset = line.offset;
        let first = m == 0;
        let last = m + 1 == file.matches.len();
        let text = StyledText::new(SharedString::from(shown.snippet.clone()))
            .with_highlights(highlights(&shown.ranges, Highlight::Match, theme));
        // Whether the note goes on above and below this line, which is
        // when a chevron has something to show.
        let (has_above, has_below) = {
            let texts = self.state.as_ref().map(|s| s.texts());
            let text = texts.as_ref().and_then(|t| t.get(&file.path));
            match text {
                Some(t) => {
                    let end = t[offset.min(t.len())..]
                        .find('\n')
                        .map_or(t.len(), |i| offset + i);
                    (offset > 0, end + 1 < t.len())
                }
                None => (false, false),
            }
        };
        let mut wash = theme.accent;
        wash.a = HOVER_ALPHA;
        let chevron_path = path.clone();
        let chevron = move |up: bool, cx: &mut Context<Self>| {
            let path = chevron_path.clone();
            div()
                .id((
                    if up {
                        "search-more-above"
                    } else {
                        "search-more-below"
                    },
                    row_ix,
                ))
                .absolute()
                .right(px(2.))
                .when(up, |d| d.top(px(2.)))
                .when(!up, |d| d.bottom(px(2.)))
                .px(px(3.))
                .py(px(1.))
                .rounded(px(4.))
                .cursor_pointer()
                .invisible()
                .group_hover("search-match", |s| s.visible())
                .hover(|s| s.bg(theme.bg_alt))
                .tooltip(tooltip("Show more context"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.grow_context(path.clone(), offset, up, cx);
                    cx.stop_propagation();
                }))
                .child(icon(
                    if up { "chevron-up" } else { "chevron-down" },
                    ICON_SIZE,
                    theme.text_faint,
                ))
        };
        let light = !theme.dark;
        div()
            .id(("search-match", row_ix))
            .group("search-match")
            .relative()
            .w_full()
            .bg(theme.bg)
            .when(first, |d| d.mt(px(4.)).rounded_t(px(4.)))
            .when(last, |d| d.mb(px(8.)).rounded_b(px(4.)))
            // The light theme rings the card with a hairline; the dark one
            // has only the hairlines between rows.
            .when(light, |d| {
                d.border_l_1().border_r_1().border_color(theme.border)
            })
            .when(light && first, |d| d.border_t_1())
            .when(light && last, |d| d.border_b_1())
            .when(!first, |d| d.border_t_1().border_color(theme.border))
            .pl(px(12.))
            .pr(px(20.))
            .py(px(8.))
            .cursor_pointer()
            .hover(move |s| s.bg(wash))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.focused = Some(row_ix);
                cx.emit(SearchEvent::Open(path.clone(), Some(offset)));
            }))
            .child(
                div()
                    .text_size(px(FONT_UI_SMALLER))
                    .line_height(relative(1.3))
                    .text_color(theme.text_muted)
                    .group_hover("search-match", |s| s.text_color(theme.text))
                    .child(text),
            )
            .children(has_above.then(|| chevron(true, cx)))
            .children(has_below.then(|| chevron(false, cx)))
            .children(focused.then(|| focus_ring(4., theme)))
            .into_any_element()
    }

    /// `search-row`: the field with its magnifier, Match case decorator and
    /// clear button, then the Search settings button, 4 px apart.
    fn render_search_row(&mut self, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let has_text = !self.input.read(cx).text().is_empty();
        // Room for the decorators the field's text must not run under:
        // the clear button only shows with text.
        let pad = if has_text { 60. } else { 32. };
        self.input
            .update(cx, |input, _| input.set_pad_right(px(pad)));
        let matching = self.matching_case;
        let mut active_wash = theme.accent;
        active_wash.a = ACTIVE_ALPHA;
        let match_case = div()
            .id("search-match-case")
            .w(px(24.))
            .h(px(20.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .cursor_pointer()
            .when(matching, |d| d.bg(active_wash))
            .hover(|s| s.bg(theme.bg_alt))
            .tooltip(tooltip("Match case"))
            .on_click(cx.listener(move |this, _, _, cx| this.set_matching_case(!matching, cx)))
            .child(icon(
                "case-sensitive",
                ICON_SIZE,
                if matching {
                    theme.accent
                } else {
                    theme.text_muted
                },
            ));
        let clear = has_text.then(|| {
            div()
                .id("search-clear")
                .w(px(28.))
                .h(px(30.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .tooltip(tooltip("Clear search"))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.set_query("", cx);
                    window.focus(&this.input.focus_handle(cx), cx);
                }))
                .child(
                    div()
                        .size(px(14.))
                        .rounded_full()
                        .bg(theme.text_muted)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon("x", 9., theme.bg_secondary)),
                )
        });
        let replace_open = self.replace_open;
        let replace_toggle = div()
            .id("search-toggle-replace")
            .flex_shrink_0()
            .w(px(28.))
            .h(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .cursor_pointer()
            .when(replace_open, |d| d.bg(active_wash))
            .hover(|s| s.bg(theme.bg_alt))
            .tooltip(tooltip("Toggle replace"))
            .on_click(cx.listener(|this, _, window, cx| {
                this.replace_open = !this.replace_open;
                if this.replace_open {
                    window.focus(&this.replace_input.focus_handle(cx), cx);
                } else {
                    window.focus(&this.input.focus_handle(cx), cx);
                }
                cx.notify();
            }))
            .child(icon(
                "arrow-left-right",
                ICON_SIZE,
                if replace_open {
                    theme.accent
                } else {
                    theme.text_muted
                },
            ));
        let settings_open = self.settings_open;
        let settings = div()
            .id("search-settings")
            .flex_shrink_0()
            .w(px(28.))
            .h(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .cursor_pointer()
            .when(settings_open, |d| d.bg(active_wash))
            .hover(|s| s.bg(theme.bg_alt))
            .tooltip(tooltip("Search settings"))
            .on_click(cx.listener(|this, _, _, cx| {
                this.settings_open = !this.settings_open;
                cx.notify();
            }))
            .child(icon(
                "sliders-horizontal",
                ICON_SIZE,
                if settings_open {
                    theme.accent
                } else {
                    theme.text_muted
                },
            ));
        div()
            .flex_shrink_0()
            .mx(px(12.))
            .mt(px(12.))
            .mb(px(8.))
            .h(px(30.))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .h(px(30.))
                    .child(self.input.clone())
                    .child(
                        div()
                            .absolute()
                            .left(px(8.))
                            .top(px(0.))
                            .h(px(30.))
                            .flex()
                            .items_center()
                            .child(icon("search", ICON_SIZE, theme.text_muted)),
                    )
                    .child(
                        div()
                            .absolute()
                            .right(px(2.))
                            .top(px(0.))
                            .h(px(30.))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(2.))
                            .child(match_case)
                            .children(clear),
                    ),
            )
            .child(replace_toggle)
            .child(settings)
    }

    fn render_replace_row(&mut self, theme: &Theme, cx: &mut Context<Self>) -> Option<gpui::Div> {
        if !self.replace_open {
            return None;
        }
        let has_text = !self.replace_input.read(cx).text().is_empty();
        let pad = if has_text { 32. } else { 8. };
        self.replace_input
            .update(cx, |input, _| input.set_pad_right(px(pad)));
        let clear = has_text.then(|| {
            div()
                .id("search-replace-clear")
                .w(px(28.))
                .h(px(30.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .tooltip(tooltip("Clear replacement"))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.replace_input.update(cx, |input, cx| input.set_text("", cx));
                    window.focus(&this.replace_input.focus_handle(cx), cx);
                }))
                .child(
                    div()
                        .size(px(14.))
                        .rounded_full()
                        .bg(theme.text_muted)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon("x", 9., theme.bg_secondary)),
                )
        });

        let replace_all_btn = div()
            .id("search-replace-all")
            .flex_shrink_0()
            .h(px(24.))
            .px(px(8.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .cursor_pointer()
            .bg(theme.bg_alt)
            .hover(|s| s.bg(theme.border))
            .tooltip(tooltip("Replace all in vault (Enter)"))
            .on_click(cx.listener(|this, _, _, cx| {
                this.replace_all(cx);
            }))
            .child(
                div()
                    .text_size(px(FONT_UI_SMALLER))
                    .text_color(theme.text)
                    .child("Replace all"),
            );

        Some(
            div()
                .flex_shrink_0()
                .mx(px(12.))
                .mb(px(8.))
                .h(px(30.))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(4.))
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .min_w_0()
                        .h(px(30.))
                        .child(self.replace_input.clone())
                        .child(
                            div()
                                .absolute()
                                .left(px(8.))
                                .top(px(0.))
                                .h(px(30.))
                                .flex()
                                .items_center()
                                .child(icon("arrow-left-right", ICON_SIZE, theme.text_muted)),
                        )
                        .child(
                            div()
                                .absolute()
                                .right(px(2.))
                                .top(px(0.))
                                .h(px(30.))
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(2.))
                                .children(clear),
                        ),
                )
                .child(replace_all_btn),
        )
    }

    /// `search-info-container`: the query as "Explain search terms" reads
    /// it, one line per node, children indented under their parent.
    fn render_explain(&self, theme: &Theme, cx: &App) -> Option<gpui::Div> {
        if !self.explain {
            return None;
        }
        let query = self.input.read(cx).text().trim().to_string();
        if query.is_empty() {
            return None;
        }
        let tree = parse(&query);
        let tree = if self.matching_case {
            Query::CaseSensitive(Box::new(tree))
        } else {
            tree
        };
        let lines: Vec<gpui::Div> = tree
            .explain()
            .lines()
            .map(|line| {
                let depth = (line.len() - line.trim_start().len()) / 2;
                div()
                    .pl(px(depth as f32 * 12.))
                    .child(SharedString::from(line.trim_start().to_string()))
            })
            .collect();
        Some(
            div()
                .flex_shrink_0()
                .px(px(16.))
                .pt(px(4.))
                .pb(px(8.))
                .flex()
                .flex_col()
                .text_size(px(FONT_UI_SMALLER))
                .line_height(relative(1.3))
                .text_color(theme.text_muted)
                .children(lines),
        )
    }

    /// `search-params`: the three toggles, 16 px in from the pane's edges.
    fn render_settings(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<gpui::Div> {
        if !self.settings_open {
            return None;
        }
        let rows = [
            ("Collapse results", self.collapse_all, 0usize),
            ("Show more context", self.extra_context, 1),
            ("Explain search terms", self.explain, 2),
        ];
        Some(
            div()
                .flex_shrink_0()
                .mx(px(16.))
                .mt(px(4.))
                .mb(px(12.))
                .flex()
                .flex_col()
                .gap(px(16.))
                .children(rows.into_iter().map(|(label, on, which)| {
                    div()
                        .h(px(19.))
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap(px(12.))
                        .child(
                            div()
                                .min_w_0()
                                .text_size(px(FONT_UI_SMALL))
                                .text_color(theme.text)
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(label),
                        )
                        .child(
                            toggle(("search-toggle", which), on, theme).on_click(cx.listener(
                                move |this, _, _, cx| match which {
                                    0 => this.set_collapse_all(!on, cx),
                                    1 => this.set_extra_context(!on, cx),
                                    _ => this.set_explain(!on, cx),
                                },
                            )),
                        )
                })),
        )
    }

    /// `search-results-info`: the count with its menu at the left, the sort
    /// order at the right, 8 px above the results.
    fn render_results_info(&mut self, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let count = self.count_text(cx);
        let count_open = self.count_menu_open;
        let count_control = div()
            .id("search-count")
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .px(px(6.))
            .py(px(4.))
            .rounded(px(4.))
            .cursor_pointer()
            .text_size(px(FONT_UI_SMALLER))
            .text_color(theme.text_muted)
            .when(count_open, |d| d.bg(theme.bg_alt))
            .hover(|s| s.bg(theme.bg_alt))
            .when(count.is_empty(), |d| d.invisible())
            .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                this.menu_at = event.position();
                this.count_menu_open = !this.count_menu_open;
                this.sort_menu_open = false;
                cx.notify();
            }))
            .child(div().whitespace_nowrap().child(count))
            .child(
                // `more-options-icon`: the ellipsis in a filled disc.
                div()
                    .size(px(14.))
                    .rounded_full()
                    .bg(theme.text_muted)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon("ellipsis", 10., theme.bg_secondary)),
            );
        let sort_open = self.sort_menu_open;
        let sort_control = div()
            .id("search-sort")
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .h(px(24.))
            .pl(px(10.))
            .pr(px(8.))
            .rounded(px(5.))
            .cursor_pointer()
            .text_size(px(FONT_UI_SMALLER))
            .text_color(theme.text_muted)
            .when(sort_open, |d| d.bg(theme.bg_alt))
            .hover(|s| s.bg(theme.bg_alt))
            .tooltip(tooltip("Change sort order"))
            .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                this.menu_at = event.position();
                this.sort_menu_open = !this.sort_menu_open;
                this.count_menu_open = false;
                cx.notify();
            }))
            .child(div().whitespace_nowrap().child(self.sort.label()))
            .child(icon("chevrons-up-down", 12., theme.text_muted));
        div()
            .flex_shrink_0()
            .px(px(12.))
            .pb(px(8.))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .child(
                div()
                    .relative()
                    .child(count_control)
                    .children(self.render_count_menu(theme, cx)),
            )
            .child(
                div()
                    .relative()
                    .child(sort_control)
                    .children(self.render_sort_menu(theme, cx)),
            )
    }

    /// The count's menu: Copy search results, Bookmark...
    fn render_count_menu(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if !self.count_menu_open {
            return None;
        }
        let items = [
            menu_item(
                "search-count-item",
                0,
                Some("copy"),
                "Copy search results",
                false,
                theme,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.count_menu_open = false;
                cx.emit(SearchEvent::CopyResults(this.copy_entries()));
                cx.notify();
            })),
            menu_item(
                "search-count-item",
                1,
                Some("bookmark"),
                "Bookmark...",
                false,
                theme,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.count_menu_open = false;
                let query = this.query(cx);
                cx.emit(SearchEvent::Bookmark(query));
                cx.notify();
            })),
        ];
        let list = menu("search-count-menu", theme)
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.count_menu_open = false;
                cx.notify();
            }))
            .children(items);
        Some(
            deferred(
                anchored()
                    .position(self.menu_at)
                    .anchor(Anchor::TopLeft)
                    .snap_to_window_with_margin(px(8.))
                    .child(list),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    /// The sort control's list: the six orders in three pairs, a check
    /// against the one in force. Obsidian's is a native `<select>`; this
    /// is the nearest thing the shell draws.
    fn render_sort_menu(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if !self.sort_menu_open {
            return None;
        }
        let current = self.sort;
        let mut list =
            menu("search-sort-menu", theme).on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.sort_menu_open = false;
                cx.notify();
            }));
        for (ix, order) in SortOrder::ALL.into_iter().enumerate() {
            if ix > 0 && ix % 2 == 0 {
                list = list.child(div().my_1().h(px(1.)).bg(theme.border));
            }
            list = list.child(
                menu_item(
                    "search-sort-item",
                    ix,
                    None,
                    order.label(),
                    order == current,
                    theme,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.set_sort(order, cx))),
            );
        }
        Some(
            deferred(
                anchored()
                    .position(self.menu_at)
                    .anchor(Anchor::TopRight)
                    .snap_to_window_with_margin(px(8.))
                    .child(list),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }
}

impl Render for SearchPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let search_row = self.render_search_row(&theme, cx);
        let replace_row = self.render_replace_row(&theme, cx);
        let explain = self.render_explain(&theme, cx);
        let settings = self.render_settings(&theme, cx);
        let info = self.render_results_info(&theme, cx);
        div()
            .flex()
            .flex_col()
            .size_full()
            .capture_action(cx.listener(Self::capture_left))
            .capture_action(cx.listener(Self::capture_right))
            .child(search_row)
            .children(replace_row)
            .children(explain)
            .children(settings)
            .child(info)
            .child(
                // `.search-result-container`: 12 px of padding; the list
                // is virtualised since a match wraps to its own height.
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .w_full()
                    .px(px(CONTAINER_PAD))
                    .pt(px(CONTAINER_PAD))
                    .child(list(self.list.clone(), cx.processor(Self::render_row)).size_full()),
            )
    }
}

/// The keyboard's ring over a row: Obsidian's `has-focus`, 2 px of the
/// focus border inside the row's own radius. Drawn over the row rather
/// than as its border so arriving does not move the text.
fn focus_ring(radius: f32, theme: &Theme) -> gpui::Div {
    div()
        .absolute()
        .inset_0()
        .rounded(px(radius))
        .border_2()
        .border_color(theme.border_focus)
}

/// A settings toggle the size of Obsidian's `checkbox-container` in a
/// pane: 34 by 19, the knob 15, the track the accent when on.
pub(crate) fn toggle(
    id: (&'static str, usize),
    on: bool,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_shrink_0()
        .relative()
        .w(px(34.))
        .h(px(19.))
        .rounded_full()
        .bg(if on { theme.accent } else { theme.tab_divider })
        .cursor_pointer()
        .child(
            div()
                .absolute()
                .top(px(2.))
                .left(px(if on { 17. } else { 2. }))
                .size(px(15.))
                .rounded_full()
                .bg(rgb(0xffffff)),
        )
}

/// A menu in the shape of Obsidian's: 8 px radius, a hairline border, 6 px
/// of padding around 13 px rows.
pub(crate) fn menu(id: &'static str, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .flex_col()
        .min_w(px(172.))
        .p(px(6.))
        .bg(theme.bg_secondary)
        .border_1()
        .border_color(theme.tab_divider)
        .rounded(px(8.))
        .shadow_lg()
        .text_size(px(FONT_UI_SMALL))
        .text_color(theme.text)
        // Keeps a click on an item from reaching what is under it.
        .occlude()
}

/// One `menu-item`: a 16 px icon slot, the title, padded 4 by 8 with a 4
/// px radius; `checked` draws a check in the slot instead.
pub(crate) fn menu_item(
    prefix: &'static str,
    ix: usize,
    icon_name: Option<&'static str>,
    title: &'static str,
    checked: bool,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let glyph: Option<gpui::Svg> = if checked {
        Some(icon("check", 14., theme.text_muted))
    } else {
        icon_name.map(|n| icon(n, ICON_SIZE, theme.text_muted))
    };
    div()
        .id((prefix, ix))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.))
        .px(px(8.))
        .py(px(4.))
        .rounded(px(4.))
        .cursor_pointer()
        .hover(|s| s.bg(theme.bg_alt))
        .child(
            div()
                .w(px(16.))
                .h(px(16.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .children(glyph),
        )
        .child(div().whitespace_nowrap().child(title))
}

pub type Inputs = (
    Vec<String>,
    Vec<Range<usize>>,
    Vec<Range<usize>>,
    Vec<TaskLine>,
);

/// Per-file search inputs derived from the cache record: tags, block
/// ranges, section ranges and task lines.
pub fn inputs(meta: &CachedMetadata, text: &str) -> Inputs {
    let tags: Vec<String> = all_tags(meta)
        .into_iter()
        .map(|t| t.to_lowercase())
        .collect();
    let blocks: Vec<Range<usize>> = meta
        .sections
        .iter()
        .map(|s| s.position.start.offset..s.position.end.offset)
        .collect();
    let mut sections = Vec::new();
    let mut at = 0;
    for h in &meta.headings {
        let start = h.position.start.offset;
        if start > at {
            sections.push(at..start);
        }
        at = start;
    }
    sections.push(at..text.len());
    let tasks: Vec<TaskLine> = meta
        .list_items
        .iter()
        .filter_map(|item| {
            item.task.map(|c| TaskLine {
                range: item.position.start.offset..item.position.end.offset,
                done: c != ' ',
            })
        })
        .collect();
    (tags, blocks, sections, tasks)
}

/// Runs `search` over every note; results in file name order.
pub fn run_search(search: &Search, state: &VaultState) -> Vec<FileResult> {
    let cache = state.cache();
    let texts = state.texts();
    let times: HashMap<String, (u64, u64)> = state
        .vault()
        .files()
        .iter()
        .map(|f| (f.path.clone(), (f.mtime, f.ctime)))
        .collect();
    let files: Vec<(&str, &CachedMetadata)> = cache.files().collect();
    let mut results: Vec<FileResult> = files
        .par_iter()
        .filter_map(|(path, meta)| {
            let text = texts.get(*path)?;
            let (tags, blocks, sections, tasks) = inputs(meta, text);
            let file = SearchFile {
                path,
                content: text,
                tags: &tags,
                frontmatter: meta.frontmatter.as_ref(),
                blocks: &blocks,
                sections: &sections,
                tasks: &tasks,
            };
            let hits = search.matches(&file)?;
            let name = path
                .rsplit('/')
                .next()
                .unwrap_or(path)
                .trim_end_matches(".md");
            let (mtime, ctime) = times.get(*path).copied().unwrap_or((0, 0));
            Some(FileResult {
                path: path.to_string(),
                name: name.to_string(),
                name_ranges: clamp(&hits.name, name.len()),
                matches: match_lines(text, &hits.content),
                hits: hits.content.clone(),
                mtime,
                ctime,
            })
        })
        .collect();
    sort_results(&mut results, SortOrder::Alphabetical);
    results
}

/// Puts results in `order`. The name orders are Obsidian's alphabetical
/// collation (numeric, locale-aware) over a file list that is already in
/// full-path-descending order, which is what `getVault().getMarkdownFiles()`
/// returns, so a tie is broken by the path **descending**: a note in a folder
/// sorts ahead of a root note of the same name, and `notes/apple` ahead of
/// `apple.md` (see [[Specification#The search collation tie, explained]]).
/// Z to A swaps the basename groups only — inside a tie the order is
/// unchanged, so the reversal is of the name comparison alone. The time
/// orders sort by the time alone and are stable, so files sharing a time keep
/// the order they had — which is what Obsidian's rows did when the sort was
/// switched.
pub fn sort_results(results: &mut [FileResult], order: SortOrder) {
    let tie = |a: &FileResult, b: &FileResult| b.path.cmp(&a.path);
    match order {
        SortOrder::Alphabetical => {
            results.sort_by(|a, b| natural_cmp(&a.name, &b.name).then_with(|| tie(a, b)));
        }
        SortOrder::AlphabeticalReverse => {
            results.sort_by(|a, b| natural_cmp(&b.name, &a.name).then_with(|| tie(a, b)));
        }
        SortOrder::ByModifiedTime => results.sort_by_key(|r| std::cmp::Reverse(r.mtime)),
        SortOrder::ByModifiedTimeReverse => results.sort_by_key(|r| r.mtime),
        SortOrder::ByCreatedTime => results.sort_by_key(|r| std::cmp::Reverse(r.ctime)),
        SortOrder::ByCreatedTimeReverse => results.sort_by_key(|r| r.ctime),
    }
}

/// The matcher reports name hits against the base name with its extension;
/// the row shows the name without it, so a hit reaching into `.md` is cut
/// there and one that lies entirely inside it is dropped.
fn clamp(ranges: &[Range<usize>], len: usize) -> Vec<Range<usize>> {
    ranges
        .iter()
        .filter(|r| r.start < len)
        .map(|r| r.start..r.end.min(len))
        .collect()
}

/// Obsidian's `search-result-file-matched-text`: the highlight behind the
/// text, the weight unchanged; a match row's text brightens to the normal
/// colour under the highlight while the rest of the row stays muted.
fn highlights(
    ranges: &[Range<usize>],
    kind: Highlight,
    theme: &Theme,
) -> Vec<(Range<usize>, HighlightStyle)> {
    ranges
        .iter()
        .map(|r| {
            (
                r.clone(),
                HighlightStyle {
                    background_color: Some(theme.highlight_bg),
                    color: match kind {
                        Highlight::Name => None,
                        Highlight::Match => Some(theme.text),
                    },
                    ..HighlightStyle::default()
                },
            )
        })
        .collect()
}

/// Zero-based line of a byte offset, like Obsidian's `Pos`. `None` when the
/// offset no longer falls on a character boundary of the text, which a stale
/// result over a changed note can do.
fn line_number(text: &str, offset: usize) -> Option<usize> {
    text.get(..offset)
        .map(|before| before.matches('\n').count())
}

/// The link style of "Copy search results".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LinkStyle {
    #[default]
    None,
    Wikilink,
    Markdown,
}

impl LinkStyle {
    pub const ALL: [LinkStyle; 3] = [LinkStyle::None, LinkStyle::Wikilink, LinkStyle::Markdown];

    pub fn label(self) -> &'static str {
        match self {
            LinkStyle::None => "None",
            LinkStyle::Wikilink => "Wikilink",
            LinkStyle::Markdown => "Markdown link",
        }
    }
}

/// The list prefix of "Copy search results".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ListPrefix {
    #[default]
    None,
    Dash,
    Asterisk,
    Numbered,
}

impl ListPrefix {
    pub const ALL: [ListPrefix; 4] = [
        ListPrefix::None,
        ListPrefix::Dash,
        ListPrefix::Asterisk,
        ListPrefix::Numbered,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ListPrefix::None => "None",
            ListPrefix::Dash => "Dash (-)",
            ListPrefix::Asterisk => "Asterisk (*)",
            ListPrefix::Numbered => "Numbered",
        }
    }
}

/// The text of the "Copy search results" modal: one line per file in list
/// order, joined by `\n` with no trailing newline. The name form is the
/// base name without extension, the path form the vault path with its
/// extension; a Markdown link's text is always the name and its target is
/// not percent-encoded. These are the 24 combinations the spec records.
pub fn copy_text(
    entries: &[CopyEntry],
    show_path: bool,
    link: LinkStyle,
    prefix: ListPrefix,
) -> String {
    entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let target = if show_path {
                e.path.as_str()
            } else {
                e.name.as_str()
            };
            let body = match link {
                LinkStyle::None => target.to_string(),
                LinkStyle::Wikilink => format!("[[{target}]]"),
                LinkStyle::Markdown => format!("[{}]({target})", e.name),
            };
            match prefix {
                ListPrefix::None => body,
                ListPrefix::Dash => format!("- {body}"),
                ListPrefix::Asterisk => format!("* {body}"),
                ListPrefix::Numbered => format!("{}. {body}", i + 1),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn regex_for_query(query_str: &str, case_sensitive: bool) -> Option<regex::Regex> {
    let q = silica_search::parse(query_str);
    find_regex_in_query(&q, case_sensitive)
}

fn find_regex_in_query(query: &Query, case_sensitive: bool) -> Option<regex::Regex> {
    match query {
        Query::Regex { pattern, flags } => {
            let ci = !case_sensitive || flags.contains('i');
            regex::RegexBuilder::new(pattern)
                .case_insensitive(ci)
                .multi_line(flags.contains('m'))
                .dot_matches_new_line(flags.contains('s'))
                .build()
                .ok()
        }
        Query::CaseSensitive(inner) => find_regex_in_query(inner, true),
        Query::CaseInsensitive(inner) => find_regex_in_query(inner, false),
        Query::All(children) | Query::Any(children) => {
            children.iter().find_map(|c| find_regex_in_query(c, case_sensitive))
        }
        Query::Field(_, inner) => find_regex_in_query(inner, case_sensitive),
        _ => None,
    }
}

pub fn apply_replacements(
    text: &str,
    hits: &[Range<usize>],
    re: Option<&regex::Regex>,
    replacement: &str,
) -> String {
    if hits.is_empty() {
        return text.to_string();
    }
    let mut sorted_hits: Vec<Range<usize>> = hits.to_vec();
    sorted_hits.sort_by_key(|r| (r.start, r.end));

    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for r in sorted_hits {
        if r.start < last || r.end < r.start || r.end > text.len() {
            continue;
        }
        if !text.is_char_boundary(r.start) || !text.is_char_boundary(r.end) {
            continue;
        }
        out.push_str(&text[last..r.start]);
        let rep = match re {
            Some(regex) => silica_text::find::replacement_for(
                text,
                &r,
                regex,
                silica_text::find::Options {
                    regex: true,
                    ..Default::default()
                },
                replacement,
            ),
            None => replacement.to_string(),
        };
        out.push_str(&rep);
        last = r.end;
    }
    if last < text.len() {
        out.push_str(&text[last..]);
    }
    out
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;

    fn result(path: &str) -> FileResult {
        let name = path
            .rsplit('/')
            .next()
            .unwrap_or(path)
            .trim_end_matches(".md");
        FileResult {
            path: path.to_string(),
            name: name.to_string(),
            name_ranges: Vec::new(),
            matches: Vec::new(),
            hits: Vec::new(),
            mtime: 0,
            ctime: 0,
        }
    }

    #[test]
    fn results_sort_the_way_obsidian_collates() {
        let mut results: Vec<FileResult> = ["Note 10.md", "9 needle.md", "Note 2.md", "_needle.md"]
            .iter()
            .map(|p| result(p))
            .collect();
        sort_results(&mut results, SortOrder::Alphabetical);
        let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["_needle", "9 needle", "Note 2", "Note 10"]);
        sort_results(&mut results, SortOrder::AlphabeticalReverse);
        let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["Note 10", "Note 2", "9 needle", "_needle"]);
        // Same basename: the tie is the full path descending, so a note in a
        // folder comes ahead of a root note of the same name. This is the
        // probe's own eight-`apple` case and its observed order.
        let paths = [
            "zzz/apple.md",
            "z/apple.md",
            "m/n/apple.md",
            "m/apple.md",
            "apple.md",
            "a/b/c/apple.md",
            "a/apple.md",
            "APPLE.md",
        ];
        let mut same: Vec<FileResult> = paths.iter().map(|p| result(p)).collect();
        same.sort_by(|x, y| x.path.cmp(&y.path));
        sort_results(&mut same, SortOrder::Alphabetical);
        let order: Vec<&str> = same.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(order, paths);
        let mut mixed: Vec<FileResult> = ["notes/b.md", "b.md", "notes/a.md", "a.md"]
            .iter()
            .map(|p| result(p))
            .collect();
        sort_results(&mut mixed, SortOrder::AlphabeticalReverse);
        let order: Vec<&str> = mixed.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(order, vec!["notes/b.md", "b.md", "notes/a.md", "a.md"]);
    }

    #[test]
    fn time_orders_keep_the_order_of_a_tie() {
        // The probe: created time put nine files in one group, and inside
        // it the rows kept the order they had before the switch.
        let mut results: Vec<FileResult> = ["c.md", "a.md", "b.md", "d.md"]
            .iter()
            .map(|p| result(p))
            .collect();
        for (i, r) in results.iter_mut().enumerate() {
            r.mtime = 100 - i as u64;
            r.ctime = if r.name == "d" { 1 } else { 5 };
        }
        sort_results(&mut results, SortOrder::ByModifiedTimeReverse);
        let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["d", "b", "a", "c"]);
        sort_results(&mut results, SortOrder::ByCreatedTime);
        let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["b", "a", "c", "d"], "the tie keeps its order");
        sort_results(&mut results, SortOrder::ByCreatedTimeReverse);
        let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["d", "b", "a", "c"]);
    }

    #[test]
    fn name_hits_are_cut_at_the_extension_and_counted_once() {
        // Ranges are against "Needle.md", the row shows "Needle".
        assert_eq!(clamp(&[0..6], 6), vec![0..6]);
        assert_eq!(clamp(&[4..9], 6), vec![4..6], "a hit over `.md` is cut");
        assert!(
            clamp(&[6..9], 6).is_empty(),
            "a hit inside `.md` is dropped"
        );
        let mut file = result("Needle.md");
        assert_eq!(file.results(), 0);
        file.name_ranges = vec![0..6];
        assert_eq!(file.results(), 1, "a name hit counts as one result");
        file.matches = vec![MatchLine {
            offset: 0,
            snippet: "needle".into(),
            ranges: vec![0..6],
        }];
        assert_eq!(file.results(), 2);
    }

    #[test]
    fn copy_text_matches_the_24_observed_forms() {
        let entries = vec![
            CopyEntry {
                path: "apple.md".into(),
                name: "apple".into(),
            },
            CopyEntry {
                path: "Apple Pie.md".into(),
                name: "Apple Pie".into(),
            },
            CopyEntry {
                path: "notes/banana.md".into(),
                name: "banana".into(),
            },
            CopyEntry {
                path: "mango.md".into(),
                name: "mango".into(),
            },
        ];
        let line = |show_path, link, prefix| {
            copy_text(&entries, show_path, link, prefix)
                .lines()
                .nth(2)
                .unwrap()
                .to_string()
        };
        assert_eq!(line(false, LinkStyle::None, ListPrefix::None), "banana");
        assert_eq!(
            line(false, LinkStyle::Wikilink, ListPrefix::None),
            "[[banana]]"
        );
        assert_eq!(
            line(false, LinkStyle::Markdown, ListPrefix::None),
            "[banana](banana)"
        );
        assert_eq!(
            line(true, LinkStyle::None, ListPrefix::None),
            "notes/banana.md"
        );
        assert_eq!(
            line(true, LinkStyle::Wikilink, ListPrefix::None),
            "[[notes/banana.md]]"
        );
        assert_eq!(
            line(true, LinkStyle::Markdown, ListPrefix::None),
            "[banana](notes/banana.md)"
        );
        assert_eq!(line(false, LinkStyle::None, ListPrefix::Dash), "- banana");
        assert_eq!(
            line(false, LinkStyle::None, ListPrefix::Asterisk),
            "* banana"
        );
        assert_eq!(
            copy_text(&entries, true, LinkStyle::Markdown, ListPrefix::Numbered),
            "1. [apple](apple.md)\n2. [Apple Pie](Apple Pie.md)\n3. [banana](notes/banana.md)\n4. [mango](mango.md)",
            "the bytes the probe read off the clipboard"
        );
    }

    #[test]
    fn test_apply_replacements_plain() {
        let text = "hello world, hello universe";
        let hits = vec![0..5, 13..18];
        let result = apply_replacements(text, &hits, None, "hi");
        assert_eq!(result, "hi world, hi universe");
    }

    #[test]
    fn test_apply_replacements_regex_captures() {
        let text = "item: 123, item: 456";
        let re = regex::Regex::new(r"item: (\d+)").unwrap();
        let hits = vec![0..9, 11..20];
        let result = apply_replacements(text, &hits, Some(&re), "id($1)");
        assert_eq!(result, "id(123), id(456)");
    }

    #[test]
    fn test_regex_for_query_parser() {
        let re = regex_for_query("/foo.*bar/i", false).expect("regex should parse");
        assert!(re.is_match("FOO-TEST-BAR"));
        assert!(regex_for_query("plain search", false).is_none());
    }
}
