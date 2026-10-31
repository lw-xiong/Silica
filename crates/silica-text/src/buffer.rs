//! The buffer: the text, its line index, the selections over it and the
//! undo history, with one method per thing the editor's keys do to them.
//!
//! The editor view owns a [`TextBuffer`] and adds the window work around
//! it — shaping, scrolling, decorations, the find panel, the properties
//! table. Everything that decides *bytes* lives here, so the Writer gate's
//! round trips can drive the same code the keys do.

use crate::autopair;
use crate::format;
use crate::lists;
use crate::plan::{indent_plan, newline_plan, shifted};
use crate::table;
use silica_markdown::HighlightColor;
use std::ops::Range;
use std::time::{Duration, Instant};
use unicode_segmentation::GraphemeCursor;

/// Typing or deleting within this of the previous step of the same kind
/// joins its undo group.
const UNDO_GROUP: Duration = Duration::from_millis(500);

/// A position in the text: zero-based line and byte offset within it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Cursor {
    pub line: usize,
    /// Byte offset within the line.
    pub col: usize,
}

/// One selection: `head` is the moving end (the cursor), `anchor` the
/// fixed one; equal when nothing is selected.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sel {
    pub anchor: usize,
    pub head: usize,
    /// Preferred x when moving between rows, in pixels. The buffer only
    /// carries it; the view measures and uses it.
    pub goal: Option<f32>,
}

impl Sel {
    pub fn at(offset: usize) -> Sel {
        Sel {
            anchor: offset,
            head: offset,
            goal: None,
        }
    }

    pub fn range(&self) -> Range<usize> {
        self.head.min(self.anchor)..self.head.max(self.anchor)
    }

    pub fn is_empty(&self) -> bool {
        self.head == self.anchor
    }
}

/// Every selection in buffer order, and which one is the primary (the one
/// the find panel, IME and scrolling follow).
#[derive(Debug, Clone, PartialEq)]
pub struct Cursors {
    pub sels: Vec<Sel>,
    pub primary: usize,
}

impl Cursors {
    /// One cursor at `offset`, nothing selected.
    pub fn at(offset: usize) -> Cursors {
        Cursors {
            sels: vec![Sel::at(offset)],
            primary: 0,
        }
    }

    /// The selections' ends, for comparing where one step ended and the
    /// next began without regard to goal x.
    pub fn positions(&self) -> Vec<(usize, usize)> {
        self.sels.iter().map(|s| (s.anchor, s.head)).collect()
    }
}

/// What kind of step an edit is, for grouping the undo history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    Typing,
    Backspace,
    Delete,
    Other,
}

/// One replacement: `old` at `at` became `new`.
#[derive(Debug)]
struct Edit {
    at: usize,
    old: String,
    new: String,
}

/// Edits undone and redone together, with the selections either side.
#[derive(Debug)]
struct Group {
    edits: Vec<Edit>,
    before: Cursors,
    after: Cursors,
    kind: EditKind,
    at: Instant,
}

#[derive(Debug, Default)]
struct History {
    undo: Vec<Group>,
    redo: Vec<Group>,
}

/// A file's text split from the bytes the buffer normalizes away.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileText {
    pub bom: bool,
    /// The file used `\r\n` on most of its lines; a save puts that ending
    /// back on every line.
    pub crlf: bool,
    /// The file used both endings, so [`FileText::join`] cannot reproduce
    /// it. The buffer keeps such a file's bytes and writes them back
    /// verbatim while nothing has changed.
    pub mixed: bool,
    /// The body with `\n` line endings.
    pub text: String,
}

impl FileText {
    pub fn parse(raw: &str) -> FileText {
        let (bom, body) = match raw.strip_prefix('\u{feff}') {
            Some(rest) => (true, rest),
            None => (false, raw),
        };
        // The dominant ending, not merely the presence of one: a single
        // stray `\r\n` in a file of 4000 Unix lines must not turn every
        // one of them on the next save.
        let breaks = body.matches('\n').count();
        let crlf_breaks = body.matches("\r\n").count();
        let crlf = crlf_breaks * 2 > breaks;
        let mixed = crlf_breaks != 0 && crlf_breaks != breaks;
        let text = if crlf_breaks > 0 {
            body.replace("\r\n", "\n")
        } else {
            body.to_string()
        };
        FileText {
            bom,
            crlf,
            mixed,
            text,
        }
    }

    pub fn join(bom: bool, crlf: bool, text: &str) -> String {
        let mut out = String::with_capacity(text.len() + 3);
        if bom {
            out.push('\u{feff}');
        }
        if crlf {
            out.push_str(&text.replace('\n', "\r\n"));
        } else {
            out.push_str(text);
        }
        out
    }
}

/// The text of one open file with everything the editor keeps about it that
/// a window is not needed for.
///
/// `text` and `line_starts` are private and always in step: the text only
/// ever changes through the editing methods, each of which rebuilds the
/// index and records the step for undo. The selections are public — moving
/// a cursor needs shaped lines, which only the view has.
#[derive(Debug, Default)]
pub struct TextBuffer {
    text: String,
    line_starts: Vec<usize>,
    /// The moving end of the primary selection; the cursor.
    pub head: usize,
    /// The fixed end of the primary selection; equals `head` when nothing
    /// is selected.
    pub anchor: usize,
    /// Preferred x when moving between rows.
    pub goal: Option<f32>,
    /// Further selections (Find all, Alt+click, add cursor above and
    /// below), in buffer order, disjoint from each other and the primary.
    pub extra: Vec<Sel>,
    /// IME composition range, absolute.
    pub marked: Option<Range<usize>>,
    /// Offsets of the closers auto-pairing planted that the cursor has not
    /// yet left the line of: typing the same character over one steps past
    /// it instead of doubling it (Specification.md, "Observed auto-pair
    /// behavior": skip-over is provenance-tracked). Edits shift them,
    /// leaving a line forgets them, and a planted closer typed over or
    /// deleted drops out of the text and the list together.
    planted: Vec<usize>,
    history: History,
    dirty: bool,
    /// A fingerprint of the bytes last known to be on disk: what was
    /// loaded, or what was last written.
    disk: u64,
    /// The file used `\r\n`; restored on save.
    crlf: bool,
    /// The file started with a byte order mark; restored on save.
    bom: bool,
    /// A file whose line endings are not all the same: its bytes and the
    /// text they loaded as. [`TextBuffer::file_contents`] writes those bytes
    /// back while the text still matches, so opening and saving such a file
    /// changes nothing; once it is edited, the dominant ending wins.
    mixed: Option<Box<MixedFile>>,
}

/// The bytes of a file the buffer cannot reproduce from its text alone.
#[derive(Debug)]
struct MixedFile {
    raw: String,
    text: String,
}

/// A cheap fingerprint of a file's bytes, so a buffer can notice the file
/// changed underneath without keeping a second copy of it.
fn fingerprint(raw: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    raw.len().hash(&mut hasher);
    raw.hash(&mut hasher);
    hasher.finish()
}

impl TextBuffer {
    /// An empty buffer, as a new editor starts.
    pub fn new() -> TextBuffer {
        let mut buf = TextBuffer::default();
        buf.rebuild_lines();
        buf
    }

    /// A buffer holding a file's bytes, the cursor at the start.
    pub fn from_file(raw: &str) -> TextBuffer {
        let mut buf = TextBuffer::new();
        buf.load(raw);
        buf
    }

    /// Replaces the text with a file's raw bytes, remembering its BOM and
    /// line endings and dropping the history. The caller places the cursor:
    /// opening a file puts it at the start, a reload clamps it where it was.
    pub fn load(&mut self, raw: &str) {
        let file = FileText::parse(raw);
        self.crlf = file.crlf;
        self.bom = file.bom;
        self.mixed = file.mixed.then(|| {
            Box::new(MixedFile {
                raw: raw.to_string(),
                text: file.text.clone(),
            })
        });
        self.text = file.text;
        self.marked = None;
        self.goal = None;
        self.extra.clear();
        self.planted.clear();
        self.dirty = false;
        self.disk = fingerprint(raw);
        self.history = History::default();
        self.rebuild_lines();
    }

    /// The bytes to write back: the text with the file's original line
    /// endings and byte order mark.
    pub fn file_contents(&self) -> String {
        match &self.mixed {
            // Untouched, and not reproducible from the text: its own bytes.
            Some(file) if file.text == self.text => file.raw.clone(),
            _ => FileText::join(self.bom, self.crlf, &self.text),
        }
    }

    /// Whether `raw` (a file's bytes) is this buffer, ignoring the BOM and
    /// line endings the buffer normalizes away.
    pub fn matches_file(&self, raw: &str) -> bool {
        FileText::parse(raw).text == self.text
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// The fingerprint of the bytes last known to be on disk.
    pub fn disk(&self) -> u64 {
        self.disk
    }

    pub fn mark_saved(&mut self) {
        self.dirty = false;
    }

    /// Records that `raw` is what the file holds now: the bytes loaded, or
    /// the bytes just written.
    pub fn mark_on_disk(&mut self, raw: &str) {
        self.disk = fingerprint(raw);
    }

    /// Whether `raw` is the bytes this buffer was loaded from or last saved
    /// as.
    ///
    /// A save that finds anything else is about to write over a change it
    /// never saw — the other app's, in the mixed use the Writer gate is
    /// about — so it must not. The fingerprint is kept rather than a second
    /// copy of the file.
    pub fn disk_unchanged(&self, raw: &str) -> bool {
        self.disk == fingerprint(raw)
    }

    /// Marks the text changed; the history steps do their own bookkeeping
    /// and call this.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// The file's line ending, for text written into it from elsewhere
    /// (the properties table serializes with it).
    pub fn crlf(&self) -> bool {
        self.crlf
    }

    pub fn bom(&self) -> bool {
        self.bom
    }

    // ----- the text ------------------------------------------------------------

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn line_starts(&self) -> &[usize] {
        &self.line_starts
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    fn rebuild_lines(&mut self) {
        self.line_starts.clear();
        self.line_starts.push(0);
        self.line_starts
            .extend(self.text.match_indices('\n').map(|(i, _)| i + 1));
    }

    /// The line's bytes without its newline.
    pub fn line_range(&self, line: usize) -> Range<usize> {
        let start = self.line_starts[line];
        let end = match self.line_starts.get(line + 1) {
            Some(next) => next - 1,
            None => self.text.len(),
        };
        start..end
    }

    pub fn line_text(&self, line: usize) -> &str {
        &self.text[self.line_range(line)]
    }

    pub fn cursor_at(&self, offset: usize) -> Cursor {
        let line = match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        Cursor {
            line,
            col: offset - self.line_starts[line],
        }
    }

    pub fn clamp_col(&self, line: usize, col: usize) -> usize {
        let text = self.line_text(line);
        let mut col = col.min(text.len());
        while !text.is_char_boundary(col) {
            col -= 1;
        }
        col
    }

    pub fn clamp_offset(&self, offset: usize) -> usize {
        floor_char_boundary(&self.text, offset)
    }

    /// One caret step back: the start of the grapheme cluster before
    /// `offset`.
    pub fn prev_grapheme_boundary(&self, offset: usize) -> usize {
        prev_grapheme_boundary(&self.text, offset)
    }

    /// One caret step forward: the end of the grapheme cluster at `offset`.
    pub fn next_grapheme_boundary(&self, offset: usize) -> usize {
        next_grapheme_boundary(&self.text, offset)
    }

    /// The caret step under the name the editor view still calls it by.
    pub fn prev_char_boundary(&self, offset: usize) -> usize {
        self.prev_grapheme_boundary(offset)
    }

    pub fn next_char_boundary(&self, offset: usize) -> usize {
        self.next_grapheme_boundary(offset)
    }

    /// The class of the grapheme cluster starting at `at`, taken from the
    /// cluster's first character so that a combining accent counts as part
    /// of the letter it sits on rather than as punctuation of its own.
    fn class_at(&self, at: usize) -> Option<u8> {
        self.text[at..].chars().next().map(char_class)
    }

    /// The class of the cluster that ends at `at`.
    fn class_before(&self, at: usize) -> Option<u8> {
        (at > 0)
            .then(|| self.class_at(self.prev_grapheme_boundary(at)))
            .flatten()
    }

    /// Word-boundary scan: whitespace, then a run of one character class.
    pub fn word_left(&self, from: usize) -> usize {
        let mut i = from;
        while i > 0 && self.class_before(i) == Some(0) {
            i = self.prev_grapheme_boundary(i);
        }
        let class = self.class_before(i);
        while i > 0 && self.class_before(i) == class {
            i = self.prev_grapheme_boundary(i);
        }
        i
    }

    pub fn word_right(&self, from: usize) -> usize {
        let mut i = from;
        let len = self.text.len();
        while i < len && self.class_at(i) == Some(0) {
            i = self.next_grapheme_boundary(i);
        }
        let class = self.class_at(i);
        while i < len && self.class_at(i) == class {
            i = self.next_grapheme_boundary(i);
        }
        i
    }

    pub fn word_at(&self, at: usize) -> Range<usize> {
        let class_before = self.class_before(at);
        let class_after = self.class_at(at);
        // Prefer the word the cursor is in or just after; whitespace selects itself.
        let class = match (class_before, class_after) {
            (_, Some(c)) if c != 0 => c,
            (Some(c), _) if c != 0 => c,
            (_, Some(c)) => c,
            (Some(c), None) => c,
            (None, None) => return at..at,
        };
        let mut start = at;
        while start > 0
            && self.class_before(start) == Some(class)
            && !self.text[..start].ends_with('\n')
        {
            start = self.prev_grapheme_boundary(start);
        }
        let mut end = at;
        while end < self.text.len()
            && self.class_at(end) == Some(class)
            && !self.text[end..].starts_with('\n')
        {
            end = self.next_grapheme_boundary(end);
        }
        start..end
    }

    // ----- selections ----------------------------------------------------------

    /// The primary cursor's line and column.
    pub fn cursor(&self) -> Cursor {
        self.cursor_at(self.head)
    }

    /// The primary selection's byte range, empty at the cursor when
    /// nothing is selected.
    pub fn selection(&self) -> Range<usize> {
        self.head.min(self.anchor)..self.head.max(self.anchor)
    }

    /// How many cursors there are (one unless Find all or Alt+click added more).
    pub fn cursor_count(&self) -> usize {
        self.extra.len() + 1
    }

    /// Every selection in buffer order, with the primary's index.
    pub fn cursors(&self) -> Cursors {
        let primary = Sel {
            anchor: self.anchor,
            head: self.head,
            goal: self.goal,
        };
        let mut sels = self.extra.clone();
        let start = primary.range().start;
        let ix = sels.partition_point(|s| s.range().start < start);
        sels.insert(ix, primary);
        Cursors { sels, primary: ix }
    }

    /// Installs `cursors`, clamped to the text, with selections that
    /// overlap (or cursors at the same place) merged into one.
    pub fn set_cursors(&mut self, cursors: Cursors) {
        let mut sels: Vec<(Sel, bool)> = cursors
            .sels
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let sel = Sel {
                    anchor: self.clamp_offset(s.anchor),
                    head: self.clamp_offset(s.head),
                    goal: s.goal,
                };
                (sel, i == cursors.primary)
            })
            .collect();
        sels.sort_by_key(|(s, _)| (s.range().start, s.range().end));
        let mut merged: Vec<(Sel, bool)> = Vec::new();
        for (s, p) in sels {
            if let Some((last, lp)) = merged.last_mut() {
                let (lr, r) = (last.range(), s.range());
                let overlaps =
                    r.start < lr.end || (lr.is_empty() && r.is_empty() && r.start == lr.start);
                if overlaps {
                    let (start, end) = (lr.start.min(r.start), lr.end.max(r.end));
                    // The primary's direction wins; otherwise the earlier one's.
                    let reversed = if p && !*lp {
                        s.head < s.anchor
                    } else {
                        last.head < last.anchor
                    };
                    *last = Sel {
                        anchor: if reversed { end } else { start },
                        head: if reversed { start } else { end },
                        goal: None,
                    };
                    *lp |= p;
                    continue;
                }
            }
            merged.push((s, p));
        }
        let primary = merged.iter().position(|(_, p)| *p).unwrap_or(0);
        let (main, _) = merged.remove(primary);
        self.head = main.head;
        self.anchor = main.anchor;
        self.goal = main.goal;
        self.extra = merged.into_iter().map(|(s, _)| s).collect();
    }

    /// Drops every cursor but the primary.
    pub fn collapse_cursors(&mut self) {
        self.extra.clear();
    }

    /// Puts one cursor at `offset` (a selection when `anchor` is given),
    /// clamped to the text, dropping the rest.
    pub fn set_cursor(&mut self, offset: usize, anchor: Option<usize>) {
        self.collapse_cursors();
        self.head = self.clamp_offset(offset);
        self.anchor = match anchor {
            Some(a) => self.clamp_offset(a),
            None => self.head,
        };
        self.goal = None;
        self.marked = None;
    }

    /// Selects the whole text, as Ctrl+A does.
    pub fn select_all(&mut self) {
        self.collapse_cursors();
        self.anchor = 0;
        self.head = self.text.len();
        self.goal = None;
    }

    /// The lines a selection touches (the cursor's line when collapsed).
    pub fn selected_lines(&self) -> Range<usize> {
        let sel = self.selection();
        let first = self.cursor_at(sel.start).line;
        let mut last = self.cursor_at(sel.end).line;
        // A selection ending at a line start does not include that line.
        if last > first && sel.end == self.line_starts[last] {
            last -= 1;
        }
        first..last + 1
    }

    /// Keeps the extra cursors in place after the primary's edit replaced
    /// `range` with `inserted` bytes.
    fn shift_extras(&mut self, range: &Range<usize>, inserted: usize) {
        if self.extra.is_empty() {
            return;
        }
        let map = |at: usize| -> usize {
            if at >= range.end {
                at - range.len() + inserted
            } else if at > range.start {
                range.start + inserted
            } else {
                at
            }
        };
        for s in &mut self.extra {
            s.head = map(s.head);
            s.anchor = map(s.anchor);
        }
        let cursors = self.cursors();
        self.set_cursors(cursors);
    }

    // ----- editing -------------------------------------------------------------

    /// Replaces `range` with `insert` at the primary cursor, records it for
    /// undo and moves the cursor after the insertion; other cursors keep
    /// their place in the text.
    pub fn edit(&mut self, range: Range<usize>, insert: &str, kind: EditKind) {
        let before = self.cursors();
        let old = self.text[range.clone()].to_string();
        self.text.replace_range(range.clone(), insert);
        self.rebuild_lines();
        self.head = range.start + insert.len();
        self.anchor = self.head;
        self.marked = None;
        self.goal = None;
        self.dirty = true;
        self.shift_extras(&range, insert.len());
        self.record(
            vec![Edit {
                at: range.start,
                old,
                new: insert.to_string(),
            }],
            kind,
            before,
        );
    }

    /// One replacement per selection, in [`TextBuffer::cursors`] order, as
    /// one undo step; every cursor collapses after its own insertion.
    /// Ranges that reach into the previous selection's replacement are cut
    /// short. False when there was nothing to do.
    pub fn edit_all(&mut self, mut edits: Vec<(Range<usize>, String)>, kind: EditKind) -> bool {
        let cursors = self.cursors();
        debug_assert_eq!(edits.len(), cursors.sels.len());
        let mut floor = 0;
        for (range, _) in &mut edits {
            range.start = range.start.max(floor).min(range.end);
            floor = range.end;
        }
        if edits.iter().all(|(r, new)| r.is_empty() && new.is_empty()) {
            return false;
        }
        let mut after = Vec::with_capacity(edits.len());
        let mut delta: isize = 0;
        for (range, new) in &edits {
            let start = (range.start as isize + delta) as usize;
            after.push(Sel::at(start + new.len()));
            delta += new.len() as isize - range.len() as isize;
        }
        let after = Cursors {
            sels: after,
            primary: cursors.primary,
        };
        self.apply_edits(edits, after, kind);
        true
    }

    /// Inserts `s` at every cursor, replacing what each has selected.
    pub fn insert_all(&mut self, s: &str, kind: EditKind) -> bool {
        let edits = self
            .cursors()
            .sels
            .iter()
            .map(|sel| (sel.range(), s.to_string()))
            .collect();
        self.edit_all(edits, kind)
    }

    /// Auto-pairs one typed character at every cursor: the closer it
    /// brings with it, the closer it steps over, or the pair a selection
    /// is wrapped in, per [`autopair`]. False when the character does
    /// nothing special anywhere, and the caller should insert it the
    /// ordinary way.
    pub fn type_pair(&mut self, typed: char, opts: autopair::Options) -> bool {
        let cursors = self.cursors();
        // A planted closer is forgotten once the cursor is on another line.
        let line = self.cursor_at(self.head).line;
        let planted = std::mem::take(&mut self.planted);
        self.planted = planted
            .into_iter()
            .filter(|&at| self.cursor_at(at).line == line)
            .collect();
        let acts: Vec<autopair::Act> = cursors
            .sels
            .iter()
            .map(|sel| {
                let planted = sel.is_empty() && self.planted.contains(&sel.head);
                autopair::act(&self.text, sel.range(), typed, opts, planted)
            })
            .collect();
        if acts.iter().all(|a| *a == autopair::Act::Insert) {
            return false;
        }
        let mut edits: Vec<(Range<usize>, String)> = Vec::new();
        let mut after = Vec::with_capacity(acts.len());
        // Each edit shifts the cursors after it, and `apply_edits` wants
        // them in post-edit coordinates.
        let mut delta: isize = 0;
        for (sel, act) in cursors.sels.iter().zip(&acts) {
            let range = sel.range();
            let shift = |at: usize| (at as isize + delta) as usize;
            match *act {
                autopair::Act::Insert | autopair::Act::Close(_) => {
                    let mut new = String::from(typed);
                    if let autopair::Act::Close(close) = *act {
                        new.push(close);
                    }
                    after.push(Sel::at(shift(range.start) + typed.len_utf8()));
                    delta += new.len() as isize - range.len() as isize;
                    edits.push((range, new));
                }
                autopair::Act::Skip => {
                    after.push(Sel::at(shift(range.start) + typed.len_utf8()));
                }
                autopair::Act::Wrap(open, close) => {
                    let (start, end) = (
                        shift(range.start) + open.len_utf8(),
                        shift(range.end) + open.len_utf8(),
                    );
                    // The selection stays over the inner text, facing the
                    // way it did.
                    let reversed = sel.head < sel.anchor;
                    after.push(Sel {
                        anchor: if reversed { end } else { start },
                        head: if reversed { start } else { end },
                        goal: None,
                    });
                    delta += (open.len_utf8() + close.len_utf8()) as isize;
                    // One edit over the whole selection rather than an
                    // insertion at each end: two selections that touch
                    // would otherwise put an opener and a closer at the
                    // same offset, where the order they land in is the
                    // sort's to decide and not this loop's.
                    let mut new = String::with_capacity(range.len() + 2);
                    new.push(open);
                    new.push_str(&self.text[range.clone()]);
                    new.push(close);
                    edits.push((range, new));
                }
            }
        }
        let after = Cursors {
            sels: after,
            primary: cursors.primary,
        };
        // A skip writes nothing, so it must not leave an undo step of its
        // own to walk back over.
        if edits.is_empty() {
            self.set_cursors(after);
            self.marked = None;
            return true;
        }
        // A wrap starts its own step; a closer joins the run of typing it
        // belongs to, so one undo takes the opener and the closer.
        let kind = if acts.iter().any(|a| matches!(a, autopair::Act::Wrap(..))) {
            EditKind::Other
        } else {
            EditKind::Typing
        };
        self.apply_edits(edits, after, kind);
        // Each closer written sits right after its cursor.
        let closers: Vec<usize> = self
            .cursors()
            .sels
            .iter()
            .zip(&acts)
            .filter(|(_, a)| matches!(a, autopair::Act::Close(_)))
            .map(|(s, _)| s.head)
            .collect();
        self.planted.extend(closers);
        true
    }

    /// Forgets the closers auto-pairing planted: the cursor has left the
    /// line, so typing one again writes it rather than stepping over it.
    pub fn forget_planted(&mut self) {
        self.planted.clear();
    }

    /// Deletes at every cursor: the selection, or what `range` gives for a
    /// collapsed one.
    pub fn delete_all(
        &mut self,
        range: impl Fn(&TextBuffer, usize) -> Range<usize>,
        kind: EditKind,
    ) -> bool {
        let edits = self
            .cursors()
            .sels
            .iter()
            .map(|sel| {
                let r = if sel.is_empty() {
                    range(self, sel.head)
                } else {
                    sel.range()
                };
                (r, String::new())
            })
            .collect();
        self.edit_all(edits, kind)
    }

    /// Applies several non-overlapping replacements as one undo step and
    /// puts the cursor at `cursor` (a selection when `anchor` is given),
    /// both in post-edit coordinates; other cursors are dropped. Ranges may
    /// come in any order. False when `edits` was empty.
    pub fn edit_many(
        &mut self,
        edits: Vec<(Range<usize>, String)>,
        cursor: usize,
        anchor: Option<usize>,
    ) -> bool {
        if edits.is_empty() {
            return false;
        }
        let after = Cursors {
            sels: vec![Sel {
                anchor: anchor.unwrap_or(cursor),
                head: cursor,
                goal: None,
            }],
            primary: 0,
        };
        self.apply_edits(edits, after, EditKind::Other);
        true
    }

    /// The text and history part of every multi-cursor edit: `after` is in
    /// post-edit coordinates.
    pub fn apply_edits(
        &mut self,
        mut edits: Vec<(Range<usize>, String)>,
        after: Cursors,
        kind: EditKind,
    ) {
        let before = self.cursors();
        // The planted closers move with the text; one inside a replaced
        // range is gone.
        self.planted = self
            .planted
            .iter()
            .filter_map(|&at| {
                let mut moved = at as isize;
                for (r, new) in &edits {
                    if at >= r.end {
                        moved += new.len() as isize - r.len() as isize;
                    } else if at > r.start || (at == r.start && !r.is_empty()) {
                        return None;
                    }
                }
                Some(moved as usize)
            })
            .collect();
        // Back to front, so earlier offsets stay valid while later text
        // moves; of two edits at one offset the replacement goes before
        // the insertion.
        edits.sort_by_key(|e| (std::cmp::Reverse(e.0.start), std::cmp::Reverse(e.0.end)));
        let mut recorded = Vec::with_capacity(edits.len());
        for (range, new) in edits {
            let old = self.text[range.clone()].to_string();
            self.text.replace_range(range.clone(), &new);
            recorded.push(Edit {
                at: range.start,
                old,
                new,
            });
        }
        self.rebuild_lines();
        self.set_cursors(after);
        self.marked = None;
        self.dirty = true;
        self.record(recorded, kind, before);
    }

    /// Records one step's edits (in the order they were applied). A typing
    /// or deleting step that starts exactly where the previous one of the
    /// same kind ended, within [`UNDO_GROUP`], joins its group.
    fn record(&mut self, edits: Vec<Edit>, kind: EditKind, before: Cursors) {
        let after = self.cursors();
        let now = Instant::now();
        self.history.redo.clear();
        if kind != EditKind::Other {
            if let Some(last) = self.history.undo.last_mut() {
                if last.kind == kind
                    && last.after.positions() == before.positions()
                    && now.duration_since(last.at) < UNDO_GROUP
                {
                    last.edits.extend(edits);
                    last.after = after;
                    last.at = now;
                    return;
                }
            }
        }
        self.history.undo.push(Group {
            edits,
            before,
            after,
            kind,
            at: now,
        });
    }

    /// Undoes one step, restoring the selections it started from. False
    /// when the history is empty.
    pub fn undo(&mut self) -> bool {
        let Some(group) = self.history.undo.pop() else {
            return false;
        };
        for e in group.edits.iter().rev() {
            self.text.replace_range(e.at..e.at + e.new.len(), &e.old);
        }
        self.rebuild_lines();
        self.set_cursors(group.before.clone());
        self.history.redo.push(group);
        self.finish_history_step();
        true
    }

    /// Redoes one undone step. False when there is none.
    pub fn redo(&mut self) -> bool {
        let Some(group) = self.history.redo.pop() else {
            return false;
        };
        for e in &group.edits {
            self.text.replace_range(e.at..e.at + e.old.len(), &e.new);
        }
        self.rebuild_lines();
        self.set_cursors(group.after.clone());
        self.history.undo.push(group);
        self.finish_history_step();
        true
    }

    /// Whether anything is on the undo stack, for a test that wants to
    /// wind a buffer all the way back.
    pub fn can_undo(&self) -> bool {
        !self.history.undo.is_empty()
    }

    fn finish_history_step(&mut self) {
        self.marked = None;
        self.goal = None;
        self.planted.clear();
        self.dirty = true;
    }

    // ----- the editing commands ------------------------------------------------

    /// Backspace: the selection at every cursor, or the character before a
    /// collapsed one.
    pub fn backspace(&mut self) -> bool {
        let kind = if self.selection().is_empty() {
            EditKind::Backspace
        } else {
            EditKind::Other
        };
        self.delete_all(|buf, at| buf.prev_char_boundary(at)..at, kind)
    }

    /// Delete: the selection at every cursor, or the character after a
    /// collapsed one.
    pub fn delete_forward(&mut self) -> bool {
        let kind = if self.selection().is_empty() {
            EditKind::Delete
        } else {
            EditKind::Other
        };
        self.delete_all(|buf, at| at..buf.next_char_boundary(at), kind)
    }

    pub fn delete_word_left(&mut self) -> bool {
        self.delete_all(|buf, at| buf.word_left(at)..at, EditKind::Other)
    }

    pub fn delete_word_right(&mut self) -> bool {
        self.delete_all(|buf, at| at..buf.word_right(at), EditKind::Other)
    }

    /// Enter: with `smart_indent_list`, a newline that continues list and
    /// quote markers and the numbering of ordered lists, and clears or
    /// outdents an empty item by one `tab_size` step. With several cursors,
    /// or a selection, a plain newline at each.
    pub fn newline(&mut self, tab_size: u8, smart_indent_list: bool) -> bool {
        let sel = self.selection();
        if !smart_indent_list || sel.start != sel.end || !self.extra.is_empty() {
            return self.insert_all("\n", EditKind::Other);
        }
        let (edits, cursor) = newline_plan(&self.text, &self.line_starts, self.head, tab_size);
        self.edit_many(edits, cursor, None)
    }

    /// Whether the cursor is in a table, which is what the table commands
    /// are offered for.
    pub fn in_table(&self) -> bool {
        table::table_at(&self.text, self.head).is_some()
    }

    /// Runs a table command at the cursor. The whole table is rewritten, as
    /// Obsidian rewrites it, and the cursor stays in the cell it was in.
    /// `false` when the cursor is not in a table or the command cannot
    /// apply, so nothing is written and no undo step is made.
    pub fn table_command(&mut self, cmd: table::TableCommand) -> bool {
        let Some(at) = table::table_at(&self.text, self.head) else {
            return false;
        };
        let Some(edit) = table::apply(&at, cmd) else {
            return false;
        };
        let cursor = at.span.start + edit.cursor;
        self.edit_many(vec![(at.span, edit.text)], cursor, None)
    }

    /// Inserts `text` at every cursor, replacing what each has selected,
    /// as one undo step: what `insert-template` and the two date commands
    /// write. Obsidian gives every cursor its own copy, which is what
    /// [`TextBuffer::insert_all`] already does.
    pub fn insert_at_cursors(&mut self, text: &str) -> bool {
        if text.is_empty() {
            return false;
        }
        self.insert_all(text, EditKind::Other)
    }

    /// `editor:insert-table`: a blank line and an empty two-by-two table,
    /// with the cursor in the first header cell.
    pub fn insert_table(&mut self) -> bool {
        let edit = table::insert_table();
        let line = self.cursor().line;
        let at = self.line_range(line).end;
        let mut text = edit.text;
        let mut cursor = at + edit.cursor;
        if !self.line_text(line).trim().is_empty() {
            // A table cannot interrupt a longer paragraph, so without a blank
            // line of its own the block stays paragraph text in Obsidian and
            // in our own parser.
            text.insert(0, '\n');
            cursor += 1;
        }
        if line + 1 < self.line_count() && self.line_text(line + 1).trim().is_empty() {
            // The line we insert before is blank already: one more newline
            // would leave two blank lines under the table.
            text.pop();
        }
        self.edit_many(vec![(at..at, text)], cursor, None)
    }

    /// Tab: indents list items and the selected lines by one step
    /// (renumbering the ordered lists it touches), and inserts the step
    /// anywhere else.
    pub fn indent(&mut self, use_tab: bool, tab_size: u8, smart_indent_list: bool) -> bool {
        let unit = lists::indent_unit(use_tab, tab_size);
        let sel = self.selection();
        let lines = self.selected_lines();
        let on_item = lists::parse_prefix(self.line_text(lines.start))
            .marker
            .is_some();
        if !self.extra.is_empty() || (sel.start == sel.end && !(smart_indent_list && on_item)) {
            return self.insert_all(&unit, EditKind::Typing);
        }
        let edits = indent_plan(
            &self.text,
            &self.line_starts,
            lines,
            &unit,
            use_tab,
            tab_size,
            true,
            smart_indent_list,
        );
        let (head, anchor) = (shifted(self.head, &edits), shifted(self.anchor, &edits));
        self.edit_many(edits, head, Some(anchor))
    }

    /// Shift+Tab: removes one indentation unit of display columns from the
    /// selected lines, clamped at zero and written back as tabs then
    /// spaces (see [`lists::outdent_edit`]), renumbering the ordered lists
    /// it touches.
    pub fn outdent(&mut self, use_tab: bool, tab_size: u8, smart_indent_list: bool) -> bool {
        let lines = self.selected_lines();
        let edits = indent_plan(
            &self.text,
            &self.line_starts,
            lines,
            "",
            use_tab,
            tab_size,
            false,
            smart_indent_list,
        );
        let (head, anchor) = (shifted(self.head, &edits), shifted(self.anchor, &edits));
        self.edit_many(edits, head, Some(anchor))
    }

    /// A formatting toggle (`editor:toggle-bold` and the rest): every
    /// selection is wrapped in `marker`, or loses the marker it is already
    /// wrapped in, as one undo step. A selection over several lines is
    /// wrapped line by line, blank lines left out, except by the comment
    /// toggle, which puts one `%%` pair around the whole of it. A bare
    /// caret takes the word it is on, or gets the pair with the caret
    /// between the halves when it is on none; the selection stays on the
    /// text it was on.
    pub fn toggle_wrap(&mut self, marker: &str) -> bool {
        self.toggle_wrap_colored(marker, None)
    }

    /// [`TextBuffer::toggle_wrap`] for `==`, with the colour one of the
    /// `editor:toggle-highlight-<colour>` commands carries: see
    /// [`format::toggle_wrap_colored`].
    pub fn toggle_wrap_colored(&mut self, marker: &str, color: Option<HighlightColor>) -> bool {
        let cursors = self.cursors();
        let m = marker.len();
        let opener = format!("{marker}{}", color.map_or("", HighlightColor::emoji));
        let mut edits: Vec<(Range<usize>, String)> = Vec::new();
        let mut after = Vec::with_capacity(cursors.sels.len());
        // `apply_edits` wants the selections in post-edit coordinates, and
        // every earlier piece's markers have moved the later ones.
        let mut delta: isize = 0;
        let mut floor = 0;
        for sel in &cursors.sels {
            let reversed = sel.head < sel.anchor;
            let pieces = self.wrap_pieces(sel.range(), marker);
            // One line keeps the selection on its text; several keep it on
            // the whole of the wrapped text, markers included, so that a
            // second press finds a pair inside each line's part.
            let outer = pieces.len() > 1;
            let mut first: Option<usize> = None;
            let mut last = 0;
            for range in pieces {
                let shift = |at: usize| (at as isize + delta) as usize;
                let (start, end) = match format::toggle_wrap_colored(
                    &self.text,
                    range.clone(),
                    marker,
                    color,
                    floor,
                ) {
                    format::Toggle::Strip(open, close) => {
                        let ends = (shift(open.start), shift(close.start) - open.len());
                        floor = close.end;
                        delta -= (open.len() + close.len()) as isize;
                        edits.push((open, String::new()));
                        edits.push((close, String::new()));
                        ends
                    }
                    format::Toggle::Wrap(r) => {
                        let ends = if outer {
                            (shift(r.start), shift(r.end) + opener.len() + m)
                        } else {
                            (shift(r.start) + opener.len(), shift(r.end) + opener.len())
                        };
                        floor = r.end;
                        delta += (opener.len() + m) as isize;
                        // One edit over the whole piece rather than an
                        // insertion at each end, for the reason
                        // `type_pair` gives: two markers at one offset
                        // would land in whichever order the sort chose.
                        let mut new = String::with_capacity(r.len() + opener.len() + m);
                        new.push_str(&opener);
                        new.push_str(&self.text[r.clone()]);
                        new.push_str(marker);
                        edits.push((r, new));
                        ends
                    }
                    format::Toggle::Recolor(emoji, with) => {
                        let grow = with.len() as isize - emoji.len() as isize;
                        let inner_start = (shift(emoji.end) as isize + grow) as usize;
                        let inner_end = (shift(range.end.max(emoji.end)) as isize + grow) as usize;
                        floor = range.end.max(emoji.end);
                        delta += grow;
                        edits.push((emoji, with.to_string()));
                        (inner_start, inner_end)
                    }
                };
                first.get_or_insert(start);
                last = end;
            }
            let (start, end) = (first.unwrap_or(sel.range().start), last);
            after.push(Sel {
                anchor: if reversed { end } else { start },
                head: if reversed { start } else { end },
                goal: None,
            });
        }
        if edits.is_empty() {
            return false;
        }
        let after = Cursors {
            sels: after,
            primary: cursors.primary,
        };
        self.apply_edits(edits, after, EditKind::Other);
        true
    }

    /// The pieces a toggle treats `range` as: the whole of it for the
    /// comment toggle; otherwise its part on each line, blank lines left
    /// out; and for a bare caret the word under it, unless the caret is
    /// between a pair of the marker, which the toggle takes out again.
    fn wrap_pieces(&self, range: Range<usize>, marker: &str) -> Vec<Range<usize>> {
        if range.is_empty() {
            let between = matches!(
                format::toggle_wrap(&self.text, range.clone(), marker, 0),
                format::Toggle::Strip(..)
            );
            if between {
                return vec![range];
            }
            let word = self.word_at(range.start);
            let on_word = self.class_at(word.start).is_some_and(|c| c != 0);
            return vec![if on_word { word } else { range }];
        }
        if marker == "%%" {
            return vec![range];
        }
        let first = self.cursor_at(range.start).line;
        let last = self.cursor_at(range.end).line;
        if first == last {
            return vec![range];
        }
        (first..=last)
            .filter_map(|l| {
                let lr = self.line_range(l);
                let piece = range.start.max(lr.start)..range.end.min(lr.end);
                (piece.start < piece.end && !self.text[piece.clone()].trim().is_empty())
                    .then_some(piece)
            })
            .collect()
    }

    /// `editor:toggle-checklist-status` (Mod+L): a checkbox on every
    /// selected line that has none, or the one it has flipped; see
    /// [`format::checklist_plan`]. The caret moves with the inserted text.
    pub fn toggle_checklist(&mut self) -> bool {
        let lines = self.selected_lines();
        let edits = format::checklist_plan(&self.text, &self.line_starts, lines);
        let (head, anchor) = (shifted(self.head, &edits), shifted(self.anchor, &edits));
        self.edit_many(edits, head, Some(anchor))
    }

    /// `editor:cycle-list-checklist`: bullet → `[ ]` → `[x]` → bullet on
    /// the selected lines; see [`format::cycle_checklist_plan`].
    pub fn cycle_checklist(&mut self) -> bool {
        let lines = self.selected_lines();
        let edits = format::cycle_checklist_plan(&self.text, &self.line_starts, lines);
        let (head, anchor) = (shifted(self.head, &edits), shifted(self.anchor, &edits));
        self.edit_many(edits, head, Some(anchor))
    }

    /// One of the six insert-block commands at the primary caret; the
    /// selection is ignored, as Obsidian ignores it. See
    /// [`format::block_insert_plan`].
    pub fn insert_block(&mut self, kind: format::BlockInsert) -> bool {
        let (edits, sel) =
            format::block_insert_plan(&self.text, &self.line_starts, self.head, kind);
        self.edit_many(edits, sel.end, Some(sel.start))
    }

    /// `editor:clear-formatting`: the markers inside every selection go,
    /// line prefixes included; see [`format::clear_formatting_plan`]. A
    /// bare caret changes nothing.
    pub fn clear_formatting(&mut self) -> bool {
        let cursors = self.cursors();
        let mut edits: Vec<(Range<usize>, String)> = Vec::new();
        let mut after = Vec::with_capacity(cursors.sels.len());
        for sel in &cursors.sels {
            let own = format::clear_formatting_plan(&self.text, &self.line_starts, sel.range());
            edits.extend(own);
            after.push(*sel);
        }
        if edits.is_empty() {
            return false;
        }
        let after = Cursors {
            sels: after
                .into_iter()
                .map(|s| Sel {
                    anchor: shifted(s.anchor, &edits),
                    head: shifted(s.head, &edits),
                    goal: None,
                })
                .collect(),
            primary: cursors.primary,
        };
        self.apply_edits(edits, after, EditKind::Other);
        true
    }

    /// `editor:set-heading-<n>`: rewrites the selected lines' leading
    /// `#`s, or removes them when `level` is 0.
    pub fn set_heading(&mut self, level: u8) -> bool {
        let lines = self.selected_lines();
        let edits = format::heading_plan(&self.text, &self.line_starts, lines, level);
        let (head, anchor) = (shifted(self.head, &edits), shifted(self.anchor, &edits));
        self.edit_many(edits, head, Some(anchor))
    }

    /// `editor:toggle-bullet-list`, `-numbered-list` and `-blockquote`:
    /// adds the prefix to the selected lines, or takes it off when they
    /// all already have it.
    pub fn toggle_line_prefix(&mut self, kind: format::LinePrefix) -> bool {
        let lines = self.selected_lines();
        let edits = format::line_prefix_plan(&self.text, &self.line_starts, lines, kind);
        let (head, anchor) = (shifted(self.head, &edits), shifted(self.anchor, &edits));
        self.edit_many(edits, head, Some(anchor))
    }

    /// `editor:delete-paragraph`: removes the block of lines around the
    /// cursor, out to the blank line either side.
    pub fn delete_paragraph(&mut self) -> bool {
        let lines = self.selected_lines();
        let span = format::paragraph_span(&self.text, &self.line_starts, lines);
        if span.is_empty() {
            return false;
        }
        let at = span.start;
        self.edit_many(vec![(span, String::new())], at, None)
    }

    /// `editor:swap-line-up` and `-down`: moves the selected lines over
    /// the line above or below, carrying the selection with them.
    pub fn swap_lines(&mut self, up: bool) -> bool {
        let lines = self.selected_lines();
        let Some((edit, shift)) = format::swap_lines_plan(&self.text, &self.line_starts, lines, up)
        else {
            return false;
        };
        let moved = |at: usize| (at as isize + shift) as usize;
        let (head, anchor) = (moved(self.head), moved(self.anchor));
        self.edit_many(vec![edit], head, Some(anchor))
    }

    /// `editor:insert-link`, `-wikilink` and `-embed`: writes the link
    /// shape around every selection, with the caret where the target is
    /// typed so the autocompletion popup opens on it.
    pub fn insert_link(&mut self, kind: format::LinkSyntax) -> bool {
        let cursors = self.cursors();
        let mut edits: Vec<(Range<usize>, String)> = Vec::new();
        let mut after = Vec::with_capacity(cursors.sels.len());
        let mut delta: isize = 0;
        for sel in &cursors.sels {
            let range = sel.range();
            let (new, caret) = format::link_insertion(&self.text[range.clone()], kind);
            let at = (range.start as isize + delta) as usize;
            after.push(Sel::at(at + caret));
            delta += new.len() as isize - range.len() as isize;
            edits.push((range, new));
        }
        let after = Cursors {
            sels: after,
            primary: cursors.primary,
        };
        self.apply_edits(edits, after, EditKind::Other);
        true
    }

    /// What each cursor copies: its selection, or its whole line when
    /// nothing is selected (what CodeMirror copies).
    pub fn copy_ranges(&self) -> Vec<Range<usize>> {
        self.cursors()
            .sels
            .iter()
            .map(|s| {
                if !s.is_empty() {
                    return s.range();
                }
                let line = self.cursor_at(s.head).line;
                let start = self.line_starts[line];
                let end = self
                    .line_starts
                    .get(line + 1)
                    .copied()
                    .unwrap_or(self.text.len());
                start..end
            })
            .collect()
    }

    /// The clipboard text for `ranges`: several cursors' texts joined with
    /// newlines.
    pub fn clipboard_text(&self, ranges: &[Range<usize>]) -> String {
        let parts: Vec<&str> = ranges.iter().map(|r| &self.text[r.clone()]).collect();
        parts.join("\n")
    }

    /// Pastes `text` at every cursor; a clipboard with one line per cursor
    /// gives each its own line, the way CodeMirror pastes a multi-cursor
    /// copy.
    pub fn paste(&mut self, text: &str) -> bool {
        let text = text.replace("\r\n", "\n");
        let cursors = self.cursors();
        let lines: Vec<&str> = text.split('\n').collect();
        let edits = cursors
            .sels
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let piece = if cursors.sels.len() > 1 && lines.len() == cursors.sels.len() {
                    lines[i].to_string()
                } else {
                    text.clone()
                };
                (s.range(), piece)
            })
            .collect();
        self.edit_all(edits, EditKind::Other)
    }
}

/// The start of the grapheme cluster before `at`, or `at` itself at the
/// start of the text. Deletion and caret motion step by cluster because a
/// combining accent, a regional-indicator flag pair and a zero-width-joiner
/// emoji are each one character to whoever typed them: stepping by code
/// point leaves a bare letter or half a flag in the note.
pub fn prev_grapheme_boundary(text: &str, at: usize) -> usize {
    let at = floor_char_boundary(text, at);
    let mut cursor = GraphemeCursor::new(at, text.len(), true);
    // The whole text is in hand, so the chunked-scan errors cannot happen
    // and `None` only means there is nothing before `at`.
    cursor.prev_boundary(text, 0).ok().flatten().unwrap_or(at)
}

/// The end of the grapheme cluster at `at`, or `at` itself at the end of
/// the text.
pub fn next_grapheme_boundary(text: &str, at: usize) -> usize {
    let at = floor_char_boundary(text, at);
    let mut cursor = GraphemeCursor::new(at, text.len(), true);
    cursor.next_boundary(text, 0).ok().flatten().unwrap_or(at)
}

/// `at`, moved back to the character boundary at or before it.
fn floor_char_boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// 0 for whitespace, 1 for word characters, 2 for punctuation.
pub fn char_class(c: char) -> u8 {
    if c.is_whitespace() {
        0
    } else if c.is_alphanumeric() || c == '_' {
        1
    } else {
        2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_classes_split_identifiers_from_punctuation() {
        assert_eq!(char_class(' '), 0);
        assert_eq!(char_class('a'), 1);
        assert_eq!(char_class('_'), 1);
        assert_eq!(char_class('['), 2);
    }

    #[test]
    fn file_text_round_trips_bom_and_line_endings() {
        for raw in [
            "plain\nlf\n",
            "\u{feff}bom\nlf",
            "crlf\r\nlines\r\n",
            "\u{feff}# both\r\n\r\ntext\r\n",
            "",
            "no trailing newline",
        ] {
            let f = FileText::parse(raw);
            assert!(!f.text.contains('\r'), "{raw:?}");
            assert_eq!(FileText::join(f.bom, f.crlf, &f.text), raw, "{raw:?}");
        }
    }

    #[test]
    fn the_dominant_line_ending_wins_and_mixed_files_are_flagged() {
        // Two CRLF breaks against one bare LF: a Windows file.
        let f = FileText::parse("a\r\nb\r\nc\nd");
        assert_eq!(f.text, "a\nb\nc\nd");
        assert!(f.crlf && f.mixed);
        assert_eq!(FileText::join(f.bom, f.crlf, &f.text), "a\r\nb\r\nc\r\nd");
        // One stray CRLF in a Unix file does not make it a Windows one.
        let f = FileText::parse("a\r\nb\nc\nd");
        assert!(!f.crlf && f.mixed);
        assert_eq!(FileText::join(f.bom, f.crlf, &f.text), "a\nb\nc\nd");
        // Uniform files are not mixed either way.
        assert!(!FileText::parse("a\nb\n").mixed);
        assert!(!FileText::parse("a\r\nb\r\n").mixed);
    }

    /// A file whose endings disagree cannot be rebuilt from the text, so an
    /// untouched buffer writes the bytes it was given. One stray `\r\n` in
    /// a file of Unix lines used to turn every line ending on the next save.
    #[test]
    fn a_mixed_file_saves_as_itself_until_it_is_edited() {
        let raw = "# Changelog\r\nfirst\nsecond\nthird\n";
        let mut buf = TextBuffer::from_file(raw);
        assert_eq!(buf.file_contents(), raw);

        buf.set_cursor(buf.len(), None);
        buf.insert_all("fourth", EditKind::Typing);
        // Edited: the dominant ending, on every line.
        assert_eq!(
            buf.file_contents(),
            "# Changelog\nfirst\nsecond\nthird\nfourth"
        );
        buf.undo();
        assert_eq!(buf.file_contents(), raw, "undo did not restore the bytes");
    }

    #[test]
    fn a_loaded_file_comes_back_as_its_own_bytes() {
        for raw in ["# A\n\ntext\n", "\u{feff}bom\r\nlines\r\n", "no newline"] {
            let buf = TextBuffer::from_file(raw);
            assert_eq!(buf.file_contents(), raw, "{raw:?}");
            assert!(!buf.is_dirty());
            assert_eq!(buf.head, 0);
        }
    }

    #[test]
    fn undo_winds_an_edited_buffer_back_to_its_bytes() {
        let raw = "- one\n- two\n";
        let mut buf = TextBuffer::from_file(raw);
        buf.set_cursor(5, None);
        buf.newline(4, true);
        buf.insert_all("three", EditKind::Typing);
        assert_eq!(buf.text(), "- one\n- three\n- two\n");
        while buf.can_undo() {
            buf.undo();
        }
        assert_eq!(buf.file_contents(), raw);
    }

    /// Two Tabs then two Shift+Tabs come back to where they started: the
    /// outdent step is the vault's `tabSize`, not four spaces, so in a
    /// two-space vault one Shift+Tab used to undo both Tabs.
    #[test]
    fn tab_and_shift_tab_round_trip_at_the_configured_width() {
        // `useTab: true`, `tabSize: 2`: a tab in, a tab out.
        let mut buf = TextBuffer::from_file("- a\n- b\n");
        buf.set_cursor(7, None);
        buf.indent(true, 2, true);
        buf.indent(true, 2, true);
        assert_eq!(buf.line_text(1), "\t\t- b");
        buf.outdent(true, 2, true);
        assert_eq!(buf.line_text(1), "\t- b");
        buf.outdent(true, 2, true);
        assert_eq!(buf.line_text(1), "- b");
        // `useTab: false`: four spaces whatever `tabSize` says, both ways.
        let mut buf = TextBuffer::from_file("- a\n- b\n");
        buf.set_cursor(7, None);
        buf.indent(false, 2, true);
        buf.indent(false, 2, true);
        assert_eq!(buf.line_text(1), "        - b");
        buf.outdent(false, 2, true);
        assert_eq!(buf.line_text(1), "    - b");
        buf.outdent(false, 2, true);
        assert_eq!(buf.line_text(1), "- b");
        // Ragged spaces under `useTab: true` come back as tabs then spaces.
        let mut buf = TextBuffer::from_file("     - b\n");
        buf.set_cursor(7, None);
        buf.outdent(true, 2, true);
        assert_eq!(buf.line_text(0), "\t - b");
    }

    /// Enter on an empty nested item outdents it by one step too.
    #[test]
    fn an_empty_nested_item_outdents_one_level() {
        let mut buf = TextBuffer::from_file("- a\n    - \n");
        buf.set_cursor(10, None);
        buf.newline(2, true);
        assert_eq!(buf.line_text(1), "  - ");
    }

    /// `editor:insert-table` has to leave a blank line above the table: a
    /// table cannot interrupt a longer paragraph, so one glued to the line
    /// above is not read as a table at all.
    #[test]
    fn an_inserted_table_is_a_block_of_its_own() {
        const TABLE: &str = "|     |     |\n| --- | --- |\n|     |     |";

        // Inside a paragraph, with a blank line and more text below.
        let mut buf = TextBuffer::from_file("# H\n\nalpha\n\n- task\n");
        buf.set_cursor(8, None);
        assert!(buf.insert_table());
        assert_eq!(buf.text(), format!("# H\n\nalpha\n\n{TABLE}\n\n- task\n"));
        // The cursor lands in the first header cell, and the table commands
        // find the table that was written.
        assert_eq!(&buf.text()[buf.head..buf.head + 5], "     ");
        assert!(buf.in_table());

        // On a blank line: the line itself is the separation.
        let mut buf = TextBuffer::from_file("# H\n\nalpha\n");
        buf.set_cursor(4, None);
        assert!(buf.insert_table());
        assert_eq!(buf.text(), format!("# H\n\n{TABLE}\n\nalpha\n"));

        // A non-blank line below keeps the newline that separates it.
        let mut buf = TextBuffer::from_file("alpha\nbeta\n");
        buf.set_cursor(2, None);
        assert!(buf.insert_table());
        assert_eq!(buf.text(), format!("alpha\n\n{TABLE}\n\nbeta\n"));

        // The last line of a file that does not end in a newline.
        let mut buf = TextBuffer::from_file("alpha");
        buf.set_cursor(5, None);
        assert!(buf.insert_table());
        assert_eq!(buf.text(), format!("alpha\n\n{TABLE}\n"));
    }

    #[test]
    fn the_line_index_follows_every_edit() {
        let mut buf = TextBuffer::from_file("a\nb\nc\n");
        assert_eq!(buf.line_count(), 4);
        buf.set_cursor(2, None);
        buf.insert_all("x\ny", EditKind::Typing);
        assert_eq!(buf.text(), "a\nx\nyb\nc\n");
        assert_eq!(buf.line_starts(), [0, 2, 4, 7, 9]);
        assert_eq!(buf.line_text(2), "yb");
        buf.undo();
        assert_eq!(buf.line_starts(), [0, 2, 4, 6]);
    }

    /// The opener and the closer are one typing step, so one undo takes
    /// both; the skip over the closer adds no step of its own.
    #[test]
    fn a_typed_bracket_brings_its_closer_and_undoes_with_it() {
        let mut buf = TextBuffer::from_file("seed\n");
        buf.set_cursor(4, None);
        assert!(buf.type_pair('(', autopair::Options::default()));
        assert_eq!(buf.text(), "seed()\n");
        assert_eq!(buf.head, 5);

        assert!(buf.type_pair(')', autopair::Options::default()));
        assert_eq!(buf.text(), "seed()\n");
        assert_eq!(buf.head, 6);

        assert!(buf.undo());
        assert_eq!(buf.text(), "seed\n");
        assert_eq!(buf.head, 4);
    }

    /// The data-losing half of the old behavior: a pair character typed
    /// over a selection used to replace it.
    #[test]
    fn a_pair_character_wraps_a_selection_and_keeps_it() {
        let mut buf = TextBuffer::from_file("alpha beta\n");
        buf.set_cursor(5, Some(0));
        assert!(buf.type_pair('*', autopair::Options::default()));
        assert_eq!(buf.text(), "*alpha* beta\n");
        // Still over `alpha`, still facing the way it was.
        assert_eq!((buf.anchor, buf.head), (1, 6));

        assert!(buf.undo());
        assert_eq!(buf.text(), "alpha beta\n");
    }

    /// Wrapping is per range, and every selection survives it.
    #[test]
    fn two_selections_are_wrapped_one_each() {
        let mut buf = TextBuffer::from_file("alpha beta\n");
        buf.set_cursor(5, Some(0));
        buf.extra = vec![Sel {
            anchor: 6,
            head: 10,
            goal: None,
        }];
        assert!(buf.type_pair('(', autopair::Options::default()));
        assert_eq!(buf.text(), "(alpha) (beta)\n");
        let after = buf.cursors();
        assert_eq!(after.sels[0].range(), 1..6);
        assert_eq!(after.sels[1].range(), 9..13);
    }

    /// Two selections that touch: the first one's closer and the second
    /// one's opener both land at the same offset, and the closer goes
    /// first.
    #[test]
    fn touching_selections_are_wrapped_without_crossing() {
        let mut buf = TextBuffer::from_file("alphabeta\n");
        buf.set_cursor(5, Some(0));
        buf.extra = vec![Sel {
            anchor: 5,
            head: 9,
            goal: None,
        }];
        assert!(buf.type_pair('(', autopair::Options::default()));
        assert_eq!(buf.text(), "(alpha)(beta)\n");
        let after = buf.cursors();
        assert_eq!(after.sels[0].range(), 1..6);
        assert_eq!(after.sels[1].range(), 8..12);
    }

    /// A selection dragged backwards keeps its direction through the wrap.
    #[test]
    fn a_backwards_selection_keeps_its_direction() {
        let mut buf = TextBuffer::from_file("alpha beta\n");
        buf.set_cursor(0, Some(5));
        assert!(buf.type_pair('_', autopair::Options::default()));
        assert_eq!(buf.text(), "_alpha_ beta\n");
        assert_eq!((buf.anchor, buf.head), (6, 1));
    }

    /// A word character ahead of the cursor suppresses the closer, so two
    /// cursors in the same word get different treatment in one keystroke.
    #[test]
    fn each_cursor_decides_for_itself_whether_to_close() {
        let mut buf = TextBuffer::from_file("ab\ncd\n");
        buf.set_cursor(0, None);
        buf.extra = vec![Sel::at(5)];
        assert!(buf.type_pair('(', autopair::Options::default()));
        // `|ab` has a word character ahead; `cd|` is at the end of a line.
        assert_eq!(buf.text(), "(ab\ncd()\n");
    }

    /// With both toggles off nothing closes, wraps or skips, and the
    /// caller is told to insert the character the ordinary way.
    #[test]
    fn nothing_pairs_when_both_settings_are_off() {
        let off = autopair::Options {
            brackets: false,
            markdown: false,
        };
        let mut buf = TextBuffer::from_file("seed\n");
        buf.set_cursor(4, None);
        assert!(!buf.type_pair('(', off));
        buf.set_cursor(4, Some(0));
        assert!(!buf.type_pair('*', off));
        assert_eq!(buf.text(), "seed\n");
    }

    #[test]
    fn bold_wraps_a_selection_and_unwraps_it_again() {
        let mut buf = TextBuffer::from_file("alpha beta\n");
        buf.set_cursor(5, Some(0));
        assert!(buf.toggle_wrap("**"));
        assert_eq!(buf.text(), "**alpha** beta\n");
        // The selection stayed on the word, so the toggle undoes itself.
        assert_eq!(buf.selection(), 2..7);
        assert!(buf.toggle_wrap("**"));
        assert_eq!(buf.text(), "alpha beta\n");
        assert_eq!(buf.selection(), 0..5);
    }

    /// With no selection an inline toggle takes the word under the caret
    /// (`one two` at ch 4 gives `one **two**`); a caret between an empty
    /// pair takes the pair out again.
    #[test]
    fn a_toggle_with_no_selection_takes_the_word_under_the_caret() {
        let mut buf = TextBuffer::from_file("one two\n");
        buf.set_cursor(4, None);
        assert!(buf.toggle_wrap("**"));
        assert_eq!(buf.text(), "one **two**\n");
        assert_eq!(buf.selection(), 6..9);
        // On whitespace there is no word: the pair goes in empty.
        let mut buf = TextBuffer::from_file("a  b\n");
        buf.set_cursor(2, None);
        assert!(buf.toggle_wrap("=="));
        assert_eq!(buf.text(), "a ==== b\n");
        assert_eq!(buf.cursor().col, 4);
        assert!(buf.toggle_wrap("=="));
        assert_eq!(buf.text(), "a  b\n");
        assert_eq!(buf.cursor().col, 2);
    }

    /// A selection over several lines is wrapped line by line, blank lines
    /// left out, except by the comment toggle; twice restores the bytes.
    #[test]
    fn a_multi_line_selection_is_wrapped_per_line() {
        for (marker, once) in [
            ("`", "`one`\n\n`two`\n`three`\n"),
            ("$", "$one$\n\n$two$\n$three$\n"),
            ("**", "**one**\n\n**two**\n**three**\n"),
            ("%%", "%%one\n\ntwo\nthree%%\n"),
        ] {
            let mut buf = TextBuffer::from_file("one\n\ntwo\nthree\n");
            buf.set_cursor(14, Some(0));
            assert!(buf.toggle_wrap(marker), "{marker}");
            assert_eq!(buf.text(), once, "{marker}");
            assert!(buf.toggle_wrap(marker), "{marker} again");
            assert_eq!(buf.text(), "one\n\ntwo\nthree\n", "{marker} restored");
            assert_eq!(buf.selection(), 0..14, "{marker} selection");
        }
        // Mid-line edges take the markers at the selection's edges.
        let mut buf = TextBuffer::from_file("one\ntwo\nthree\n");
        buf.set_cursor(10, Some(1));
        assert!(buf.toggle_wrap("`"));
        assert_eq!(buf.text(), "o`ne`\n`two`\n`th`ree\n");
    }

    /// The six colour commands, through the buffer: the wrap and its undo.
    #[test]
    fn a_colour_highlight_wraps_with_its_emoji_and_recolours_in_place() {
        let mut buf = TextBuffer::from_file("word\n");
        buf.set_cursor(4, Some(0));
        assert!(buf.toggle_wrap_colored("==", Some(HighlightColor::Red)));
        assert_eq!(buf.text(), "==\u{1f534}word==\n");
        assert_eq!(buf.selection(), 6..10);
        assert!(buf.toggle_wrap_colored("==", Some(HighlightColor::Blue)));
        assert_eq!(buf.text(), "==\u{1f535}word==\n");
        assert_eq!(buf.selection(), 6..10);
        assert!(buf.toggle_wrap_colored("==", Some(HighlightColor::Blue)));
        assert_eq!(buf.text(), "word\n");
        assert_eq!(buf.selection(), 0..4);
        // Per line, as the plain toggles.
        let mut buf = TextBuffer::from_file("one\ntwo\n");
        buf.set_cursor(7, Some(0));
        assert!(buf.toggle_wrap_colored("==", Some(HighlightColor::Yellow)));
        assert_eq!(buf.text(), "==\u{1f7e1}one==\n==\u{1f7e1}two==\n");
    }

    #[test]
    fn a_toggle_wraps_every_cursor_in_one_undo_step() {
        let mut buf = TextBuffer::from_file("one two\n");
        buf.set_cursor(3, Some(0));
        buf.extra = vec![Sel {
            anchor: 4,
            head: 7,
            goal: None,
        }];
        assert!(buf.toggle_wrap("*"));
        assert_eq!(buf.text(), "*one* *two*\n");
        assert!(buf.undo());
        assert_eq!(buf.text(), "one two\n");
    }

    #[test]
    fn the_checkbox_toggle_writes_the_bytes_the_click_writes() {
        let mut buf = TextBuffer::from_file("- [ ] a\n- [x] b\n- [?] c\n");
        buf.set_cursor(2, None);
        assert!(buf.toggle_checklist());
        assert_eq!(buf.text(), "- [x] a\n- [x] b\n- [?] c\n");
        assert_eq!(buf.cursor(), Cursor { line: 0, col: 2 });
        // A selection over the other two flips each of them.
        buf.set_cursor(8, Some(23));
        assert!(buf.toggle_checklist());
        assert_eq!(buf.text(), "- [x] a\n- [ ] b\n- [ ] c\n");
    }

    /// Mod+L on a line without a checkbox adds one, and the caret moves
    /// with the inserted text: `plain text` at ch 3 ends at ch 9.
    #[test]
    fn a_line_without_a_checkbox_gains_one_under_the_caret() {
        let mut buf = TextBuffer::from_file("plain text\n");
        buf.set_cursor(3, None);
        assert!(buf.toggle_checklist());
        assert_eq!(buf.text(), "- [ ] plain text\n");
        assert_eq!(buf.cursor(), Cursor { line: 0, col: 9 });
        assert!(buf.toggle_checklist());
        assert_eq!(buf.text(), "- [x] plain text\n");
        assert!(buf.toggle_checklist());
        assert_eq!(buf.text(), "- [ ] plain text\n");
    }

    #[test]
    fn the_cycle_command_and_the_insert_blocks_go_through_the_buffer() {
        let mut buf = TextBuffer::from_file("item\n");
        buf.set_cursor(2, None);
        assert!(buf.cycle_checklist());
        assert_eq!(buf.text(), "- item\n");
        assert_eq!(buf.cursor().col, 4);
        assert!(buf.cycle_checklist());
        assert_eq!(buf.text(), "- [ ] item\n");
        assert!(buf.undo());
        assert_eq!(buf.text(), "- item\n");

        let mut buf = TextBuffer::from_file("alpha beta\n");
        buf.set_cursor(5, Some(0));
        assert!(buf.insert_block(format::BlockInsert::Callout));
        assert_eq!(buf.text(), "> [!NOTE]\n> alpha beta\n");
        assert_eq!(buf.selection(), 4..8);
        assert_eq!(&buf.text()[buf.selection()], "NOTE");

        let mut buf = TextBuffer::from_file("body text");
        buf.set_cursor(9, None);
        assert!(buf.insert_block(format::BlockInsert::Footnote));
        assert_eq!(buf.text(), "body text[^1]\n\n[^1]: \n");
        assert_eq!(buf.head, 13);
    }

    #[test]
    fn clear_formatting_needs_a_selection_and_keeps_it() {
        let mut buf = TextBuffer::from_file("**bold** _it_\n");
        buf.set_cursor(3, None);
        assert!(!buf.clear_formatting());
        buf.set_cursor(13, Some(0));
        assert!(buf.clear_formatting());
        assert_eq!(buf.text(), "bold it\n");
        assert_eq!(buf.selection(), 0..7);
    }

    #[test]
    fn heading_keeps_the_cursor_on_the_same_word() {
        let mut buf = TextBuffer::from_file("alpha\n");
        buf.set_cursor(5, None);
        assert!(buf.set_heading(2));
        assert_eq!(buf.text(), "## alpha\n");
        assert_eq!(buf.cursor().col, 8);
        assert!(buf.set_heading(0));
        assert_eq!(buf.text(), "alpha\n");
        assert_eq!(buf.cursor().col, 5);
    }

    #[test]
    fn a_bullet_toggle_covers_the_selected_lines() {
        let mut buf = TextBuffer::from_file("one\ntwo\n");
        buf.set_cursor(0, Some(7));
        assert!(buf.toggle_line_prefix(format::LinePrefix::Bullet));
        assert_eq!(buf.text(), "- one\n- two\n");
        assert!(buf.toggle_line_prefix(format::LinePrefix::Ordered));
        assert_eq!(buf.text(), "1. one\n2. two\n");
        assert!(buf.toggle_line_prefix(format::LinePrefix::Ordered));
        assert_eq!(buf.text(), "one\ntwo\n");
    }

    #[test]
    fn delete_paragraph_takes_the_block_around_the_cursor() {
        let mut buf = TextBuffer::from_file("one\n\ntwo\nthree\n\nfour\n");
        buf.set_cursor(7, None);
        assert!(buf.delete_paragraph());
        assert_eq!(buf.text(), "one\n\n\nfour\n");
        assert_eq!(buf.head, 5);
    }

    #[test]
    fn moving_a_line_carries_the_cursor_with_it() {
        let mut buf = TextBuffer::from_file("a\nbb\nc\n");
        buf.set_cursor(3, None);
        assert!(buf.swap_lines(true));
        assert_eq!(buf.text(), "bb\na\nc\n");
        assert_eq!(buf.head, 1);
        assert!(buf.swap_lines(false));
        assert_eq!(buf.text(), "a\nbb\nc\n");
        assert_eq!(buf.head, 3);
        // The first line has nothing above it.
        buf.set_cursor(0, None);
        assert!(!buf.swap_lines(true));
    }

    #[test]
    fn insert_link_puts_the_caret_where_the_target_goes() {
        let mut buf = TextBuffer::from_file("see x\n");
        buf.set_cursor(4, None);
        assert!(buf.insert_link(format::LinkSyntax::Markdown));
        assert_eq!(buf.text(), "see []()x\n");
        assert_eq!(buf.head, 5);
        assert!(buf.undo());
        buf.set_cursor(4, Some(3));
        assert!(buf.insert_link(format::LinkSyntax::Wikilink));
        assert_eq!(buf.text(), "see[[ ]]x\n");
        assert_eq!(buf.head, 6);
    }

    /// A combining accent, a regional-indicator flag pair and a
    /// zero-width-joiner emoji are each one character to whoever typed
    /// them. Stepping by code point left a bare `e`, half a flag or a
    /// dangling joiner in the note.
    #[test]
    fn deletion_and_caret_motion_take_a_grapheme_cluster_whole() {
        for cluster in [
            "e\u{301}",
            "\u{1f1eb}\u{1f1f7}",
            "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}\u{200d}\u{1f466}",
            "x",
        ] {
            let text = format!("a{cluster}b\n");
            let after = 1 + cluster.len();

            let mut buf = TextBuffer::from_file(&text);
            buf.set_cursor(after, None);
            assert!(buf.backspace());
            assert_eq!(buf.text(), "ab\n", "backspace over {cluster:?}");

            let mut buf = TextBuffer::from_file(&text);
            buf.set_cursor(1, None);
            assert!(buf.delete_forward());
            assert_eq!(buf.text(), "ab\n", "delete over {cluster:?}");

            // The caret stops either side of the cluster, never inside it.
            let buf = TextBuffer::from_file(&text);
            assert_eq!(
                buf.next_grapheme_boundary(1),
                after,
                "right over {cluster:?}"
            );
            assert_eq!(
                buf.prev_grapheme_boundary(after),
                1,
                "left over {cluster:?}"
            );
            assert_eq!(buf.prev_grapheme_boundary(0), 0);
            assert_eq!(buf.next_grapheme_boundary(buf.len()), buf.len());
        }
    }

    /// Ctrl+arrow and double-click keep an accented letter in its word:
    /// classifying a cluster by its last code point made the combining
    /// mark punctuation and broke the word there.
    #[test]
    fn word_motion_keeps_an_accented_letter_in_its_word() {
        let buf = TextBuffer::from_file("one cafe\u{301} two\n");
        let end = "one cafe\u{301}".len();
        assert_eq!(buf.word_left(end), 4);
        assert_eq!(buf.word_right(4), end);
        assert_eq!(buf.word_at(5), 4..end);
    }
}
