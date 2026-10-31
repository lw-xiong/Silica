//! Heading and list folding: which lines fold, what a fold hides, how the
//! editor and the reading view share one set of folds, and the record kept
//! outside the vault so a fold survives a reload and a relaunch.
//!
//! Every rule here is from `docs/silica/Specification.md`, "Observed heading
//! fold behavior": a heading folds through the line before the next heading
//! of the same or a higher level, trailing blank lines included, and the
//! last heading folds to the end of the file; a list parent folds its own
//! lines and not the blank line after them; a heading in a blockquote or a
//! callout, a setext underline and a heading on the last line of a file with
//! no trailing newline do not fold. Nothing is written to the note. Obsidian
//! keeps the record in `localStorage` under `<app id>-note-fold-<path>` as
//! `{"folds":[{"from","to"}],"lines"}`; Silica keeps the same record in a
//! file under its own configuration folder.

use silica_markdown::{plain_text, walk_blocks, BlockKind, Document, LineIndex};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What kind of line a fold starts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FoldKind {
    /// A heading of this level.
    Heading(u8),
    /// A list item with lines under it.
    List,
}

/// A line that can fold, and the last line its fold hides (inclusive).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Foldable {
    pub kind: FoldKind,
    pub line: usize,
    pub end: usize,
    /// The heading's plain text, or the list item's first line trimmed:
    /// what a fold is remembered by across edits that move the line.
    pub text: String,
}

/// Which of the two `app.json` toggles are on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FoldOptions {
    /// `foldHeading`, default on.
    pub headings: bool,
    /// `foldIndent`, default on: list folding.
    pub lists: bool,
}

impl Default for FoldOptions {
    fn default() -> FoldOptions {
        FoldOptions {
            headings: true,
            lists: true,
        }
    }
}

/// Every line of `text` that can fold, in line order, from the one parse.
pub fn foldables(doc: &Document<'_>, text: &str, opts: FoldOptions) -> Vec<Foldable> {
    let index = LineIndex::new(text);
    let line_of = |offset: usize| index.pos(offset).line as usize;
    let last_line = index.line_count() - 1;
    let mut out = Vec::new();
    if opts.headings {
        // Only top-level headings: one in a blockquote or a callout was
        // observed not to fold.
        let headings: Vec<(usize, u8, String)> = doc
            .blocks
            .iter()
            .filter_map(|b| match &b.kind {
                BlockKind::Heading { level, content, .. } => Some((
                    line_of(b.span.start),
                    *level,
                    plain_text(content).trim().to_string(),
                )),
                _ => None,
            })
            .collect();
        for (i, (line, level, text)) in headings.iter().enumerate() {
            let end = headings[i + 1..]
                .iter()
                .find(|(_, l, _)| l <= level)
                .map(|(next, _, _)| next - 1)
                .unwrap_or(last_line);
            if end > *line {
                out.push(Foldable {
                    kind: FoldKind::Heading(*level),
                    line: *line,
                    end,
                    text: text.clone(),
                });
            }
        }
    }
    if opts.lists {
        walk_blocks(&doc.blocks, &mut |b| {
            let BlockKind::List { items, .. } = &b.kind else {
                return;
            };
            for item in items {
                let line = line_of(item.span.start);
                let end = line_of(item.span.end.saturating_sub(1).max(item.span.start));
                if end > line {
                    let first = index.line_start(line);
                    let stop = text[first..].find('\n').map_or(text.len(), |n| first + n);
                    out.push(Foldable {
                        kind: FoldKind::List,
                        line,
                        end,
                        text: text[first..stop].trim().to_string(),
                    });
                }
            }
        });
    }
    out.sort_by_key(|f| (f.line, f.kind));
    out
}

/// How a fold is remembered: the kind and text of its line, and which of
/// the lines with that kind and text it is. An edit above the line moves
/// it without changing the key; an edit on the line changes the key, and
/// [`Folds::resolve`] then finds the fold by its line instead.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FoldKey {
    pub kind: FoldKind,
    pub text: String,
    pub nth: usize,
}

/// The key of the `ix`th foldable.
pub fn key_of(foldables: &[Foldable], ix: usize) -> FoldKey {
    let f = &foldables[ix];
    let nth = foldables[..ix]
        .iter()
        .filter(|o| o.kind == f.kind && o.text == f.text)
        .count();
    FoldKey {
        kind: f.kind,
        text: f.text.clone(),
        nth,
    }
}

/// The keys of a run of headings in order, for a view that has the
/// headings but no parse: the reading view's blocks.
pub fn heading_keys<'a>(headings: impl Iterator<Item = (u8, &'a str)>) -> Vec<FoldKey> {
    let mut seen: Vec<(u8, &str)> = Vec::new();
    headings
        .map(|(level, text)| {
            let nth = seen
                .iter()
                .filter(|(l, t)| *l == level && *t == text)
                .count();
            seen.push((level, text));
            FoldKey {
                kind: FoldKind::Heading(level),
                text: text.to_string(),
                nth,
            }
        })
        .collect()
}

/// One fold: its key, and the line it last resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Fold {
    key: FoldKey,
    line: usize,
}

/// The folds of one note, as the editor keeps them: a set of keys that is
/// resolved against the current foldables after every parse, so an edit
/// that moves a heading moves its fold with it and a heading that goes
/// takes its fold away.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Folds {
    entries: Vec<Fold>,
}

impl Folds {
    pub fn from_keys(keys: impl IntoIterator<Item = FoldKey>) -> Folds {
        Folds {
            entries: keys
                .into_iter()
                .map(|key| Fold {
                    key,
                    line: usize::MAX,
                })
                .collect(),
        }
    }

    /// The keys, for the other view and the record.
    pub fn keys(&self) -> Vec<FoldKey> {
        self.entries.iter().map(|f| f.key.clone()).collect()
    }

    /// The folds from a stored record: each `from` line that is a foldable
    /// line folds; the stored `to` is advisory and the extent is the
    /// foldable's own, as the editor was observed re-deriving it.
    pub fn from_record(foldables: &[Foldable], froms: impl IntoIterator<Item = usize>) -> Folds {
        let mut folds = Folds::default();
        for from in froms {
            if let Some(ix) = foldables.iter().position(|f| f.line == from) {
                folds.insert(foldables, ix);
            }
        }
        folds
    }

    fn insert(&mut self, foldables: &[Foldable], ix: usize) {
        let key = key_of(foldables, ix);
        if !self.entries.iter().any(|f| f.key == key) {
            self.entries.push(Fold {
                key,
                line: foldables[ix].line,
            });
        }
    }

    fn find(&self, foldables: &[Foldable], key: &FoldKey) -> Option<usize> {
        (0..foldables.len()).find(|&ix| key_of(foldables, ix) == *key)
    }

    /// Settles every fold against `foldables`: by key, or by the line it
    /// was last on when the key no longer matches (the heading was edited),
    /// in which case the key is updated; a fold that resolves neither way is
    /// dropped. Returns the hidden line ranges, `(first hidden, last
    /// hidden)` inclusive, in line order and non-overlapping where nested.
    pub fn resolve(&mut self, foldables: &[Foldable]) -> Vec<(usize, usize)> {
        let mut ranges = Vec::new();
        let mut kept: Vec<Fold> = Vec::new();
        for mut fold in std::mem::take(&mut self.entries) {
            let ix = match self.find(foldables, &fold.key) {
                Some(ix) => ix,
                None => match foldables
                    .iter()
                    .position(|f| f.line == fold.line && f.kind == fold.key.kind)
                {
                    Some(ix) => {
                        fold.key = key_of(foldables, ix);
                        ix
                    }
                    None => continue,
                },
            };
            let f = &foldables[ix];
            fold.line = f.line;
            if kept.iter().any(|k: &Fold| k.key == fold.key) {
                continue;
            }
            ranges.push((f.line + 1, f.end));
            kept.push(fold);
        }
        self.entries = kept;
        ranges.sort_unstable();
        ranges
    }

    /// Whether the foldable at `line` is folded.
    pub fn is_folded(&self, line: usize) -> bool {
        self.entries.iter().any(|f| f.line == line)
    }

    /// `editor:toggle-fold` on `line`: folds the foldable starting there,
    /// or unfolds it. False when the line starts no fold, which Obsidian
    /// answers by doing nothing.
    pub fn toggle(&mut self, foldables: &[Foldable], line: usize) -> bool {
        let Some(ix) = foldables.iter().position(|f| f.line == line) else {
            return false;
        };
        let key = key_of(foldables, ix);
        match self.entries.iter().position(|f| f.key == key) {
            Some(at) => {
                self.entries.remove(at);
            }
            None => self.insert(foldables, ix),
        }
        true
    }

    /// `editor:fold-more`: folds the innermost unfolded foldable around
    /// `line` — the caret's own section first, then each enclosing one.
    pub fn fold_more(&mut self, foldables: &[Foldable], line: usize) -> bool {
        let enclosing: Vec<usize> = (0..foldables.len())
            .filter(|&ix| foldables[ix].line <= line && line <= foldables[ix].end)
            .collect();
        for &ix in enclosing.iter().rev() {
            let key = key_of(foldables, ix);
            if !self.entries.iter().any(|f| f.key == key) {
                self.insert(foldables, ix);
                return true;
            }
        }
        false
    }

    /// `editor:fold-less`: unfolds the outermost folded foldable around
    /// `line`.
    pub fn fold_less(&mut self, foldables: &[Foldable], line: usize) -> bool {
        let outer = (0..foldables.len())
            .filter(|&ix| foldables[ix].line <= line && line <= foldables[ix].end)
            .find(|&ix| {
                let key = key_of(foldables, ix);
                self.entries.iter().any(|f| f.key == key)
            });
        let Some(ix) = outer else {
            return false;
        };
        let key = key_of(foldables, ix);
        self.entries.retain(|f| f.key != key);
        true
    }

    /// `editor:fold-all`.
    pub fn fold_all(&mut self, foldables: &[Foldable]) {
        for ix in 0..foldables.len() {
            self.insert(foldables, ix);
        }
    }

    /// `editor:unfold-all`.
    pub fn unfold_all(&mut self) {
        self.entries.clear();
    }

    /// Unfolds whatever hides `line`: a link into a folded section expands
    /// it rather than scrolling to the placeholder.
    pub fn unfold_hiding(&mut self, foldables: &[Foldable], line: usize) -> bool {
        let before = self.entries.len();
        let hiding: Vec<FoldKey> = (0..foldables.len())
            .filter(|&ix| foldables[ix].line < line && line <= foldables[ix].end)
            .map(|ix| key_of(foldables, ix))
            .collect();
        self.entries.retain(|f| !hiding.contains(&f.key));
        self.entries.len() != before
    }

    /// The record's `from`/`to` pairs for the folds that resolve.
    pub fn record(&self, foldables: &[Foldable]) -> Vec<(usize, usize)> {
        self.entries
            .iter()
            .filter_map(|fold| {
                let ix = self.find(foldables, &fold.key)?;
                Some((foldables[ix].line, foldables[ix].end))
            })
            .collect()
    }
}

/// Whether `line` is inside one of `hidden`, the ranges [`Folds::resolve`]
/// returns.
pub fn is_hidden(hidden: &[(usize, usize)], line: usize) -> bool {
    hidden.iter().any(|&(a, b)| a <= line && line <= b)
}

/// The end of the hidden run `line` is in, when it is in one.
pub fn hidden_end(hidden: &[(usize, usize)], line: usize) -> Option<usize> {
    hidden
        .iter()
        .filter(|&&(a, b)| a <= line && line <= b)
        .map(|&(_, b)| b)
        .max()
}

// ----- the record outside the vault ------------------------------------------

/// One note's record, in Obsidian's shape.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    #[serde(default)]
    pub folds: Vec<FoldRange>,
    #[serde(default)]
    pub lines: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoldRange {
    pub from: usize,
    pub to: usize,
}

/// The properties fold's record, which Obsidian keeps as `{0,0}`.
pub const PROPERTIES_RANGE: FoldRange = FoldRange { from: 0, to: 0 };

/// The fold records of one vault's notes, by vault-relative path, kept in
/// `note-fold/<vault id>.json` under Silica's configuration folder — the
/// place `silica.json` lives — and never in the vault.
#[derive(Debug, Clone, Default)]
pub struct FoldStore {
    path: Option<PathBuf>,
    notes: BTreeMap<String, Record>,
}

impl FoldStore {
    /// The store for the vault at `root`, read from disk; a store that has
    /// nowhere to live (no configuration folder) keeps records for the
    /// session only.
    pub fn open(root: &Path) -> FoldStore {
        let path = silica_vault::registry::config_dir().map(|dir| {
            dir.join("note-fold")
                .join(format!("{}.json", vault_id(root)))
        });
        let notes = path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        FoldStore { path, notes }
    }

    pub fn get(&self, path: &str) -> Option<&Record> {
        self.notes.get(path)
    }

    /// Records `record` for `path` and writes the store; an empty record
    /// removes the note's entry.
    pub fn put(&mut self, path: &str, record: Record) {
        let changed = if record.folds.is_empty() {
            self.notes.remove(path).is_some()
        } else {
            self.notes.get(path) != Some(&record) && {
                self.notes.insert(path.to_string(), record);
                true
            }
        };
        if changed {
            self.write();
        }
    }

    /// A rename or move carries the record with the note.
    pub fn rename(&mut self, from: &str, to: &str) {
        if let Some(record) = self.notes.remove(from) {
            self.notes.insert(to.to_string(), record);
            self.write();
        }
    }

    fn write(&self) {
        let Some(path) = &self.path else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(&self.notes) {
            let _ = std::fs::write(path, text);
        }
    }
}

/// Sixteen hex digits for a vault folder, the shape of the registry's ids.
fn vault_id(root: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    root.hash(&mut h);
    format!("{:016x}", h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The probe's 33-line note, by shape: frontmatter, a preamble, `# H1
    /// Top` on 7, `## Section A` on 11, `### Sub A1` on 15, `## Section B`
    /// on 19 with a list parent on 23, `## Section C` on 27 with a footnote
    /// definition on 31 and the empty line 32.
    const NOTE: &str = "---\ntitle: Fold Probe\ntags:\n  - a\n---\nPREAMBLEBODY text\n\n# H1 Top\nTOPBODY\n\n\n## Section A\nbody a\n\n\n### Sub A1\nsub body\n\n\n## Section B\nbody b\n\n\n- list parent\n  - child one\n  - child two\n\n## Section C\nbody c\n\n\n[^1]: note\n";

    fn all() -> Vec<Foldable> {
        let doc = silica_markdown::parse(NOTE);
        foldables(&doc, NOTE, FoldOptions::default())
    }

    #[test]
    fn the_probes_extents() {
        assert_eq!(NOTE.split('\n').count(), 33);
        let got: Vec<(usize, usize)> = all().iter().map(|f| (f.line, f.end)).collect();
        assert_eq!(
            got,
            vec![(7, 32), (11, 18), (15, 18), (19, 26), (23, 25), (27, 32)]
        );
        assert_eq!(all()[0].text, "H1 Top");
        assert_eq!(all()[4].text, "- list parent");
    }

    #[test]
    fn the_edge_headings() {
        let f = |text: &str| {
            let doc = silica_markdown::parse(text);
            foldables(&doc, text, FoldOptions::default())
                .iter()
                .map(|f| (f.line, f.end))
                .collect::<Vec<_>>()
        };
        // A heading on line 0, and a setext heading by its text line.
        assert_eq!(f("# Line zero\na\nb\n"), vec![(0, 3)]);
        assert_eq!(f("Title\n=====\nbody\n"), vec![(0, 3)]);
        // In a callout or a blockquote: no.
        assert!(f("> ## Quoted\n> body\n").is_empty());
        assert!(f("> [!note] T\n> ## In a callout\n> body\n").is_empty());
        // The last line of a file with no trailing newline.
        assert!(f("body\n## Empty Last Section").is_empty());
        // Two headings of one level: the first stops at the second.
        assert_eq!(f("## A\na\n## B\nb\n"), vec![(0, 1), (2, 4)]);
    }

    #[test]
    fn the_toggles_fold_and_unfold_by_key() {
        let fs = all();
        let mut folds = Folds::default();
        assert!(folds.toggle(&fs, 11));
        assert_eq!(folds.resolve(&fs), vec![(12, 18)]);
        assert!(!folds.toggle(&fs, 5), "a paragraph starts no fold");
        assert!(folds.toggle(&fs, 11));
        assert!(folds.resolve(&fs).is_empty());
        // fold-more walks outward from the caret's section, fold-less back.
        let mut folds = Folds::default();
        assert!(folds.fold_more(&fs, 12));
        assert_eq!(folds.record(&fs), vec![(11, 18)]);
        assert!(folds.fold_more(&fs, 12));
        assert_eq!(folds.record(&fs), vec![(11, 18), (7, 32)]);
        assert!(!folds.fold_more(&fs, 12));
        assert!(folds.fold_less(&fs, 12));
        assert_eq!(folds.record(&fs), vec![(11, 18)]);
        assert!(folds.fold_less(&fs, 12));
        assert!(!folds.fold_less(&fs, 12));
        // fold-all and unfold-all.
        folds.fold_all(&fs);
        assert_eq!(folds.record(&fs).len(), 6);
        folds.unfold_all();
        assert!(folds.keys().is_empty());
    }

    #[test]
    fn a_fold_follows_its_heading_through_edits() {
        let fs = all();
        let mut folds = Folds::default();
        folds.toggle(&fs, 19);
        // A line inserted above: the key still matches, the line moves.
        let moved = NOTE.replacen("PREAMBLEBODY", "x\nPREAMBLEBODY", 1);
        let doc = silica_markdown::parse(&moved);
        let fs2 = foldables(&doc, &moved, FoldOptions::default());
        assert_eq!(folds.resolve(&fs2), vec![(21, 27)]);
        // The heading's text edited in place: found by its line, re-keyed.
        let renamed = moved.replacen("## Section B", "## Section Bee", 1);
        let doc = silica_markdown::parse(&renamed);
        let fs3 = foldables(&doc, &renamed, FoldOptions::default());
        assert_eq!(folds.resolve(&fs3), vec![(21, 27)]);
        assert_eq!(folds.keys()[0].text, "Section Bee");
        // The heading gone: the fold goes with it.
        let gone = renamed.replacen("## Section Bee\n", "", 1);
        let doc = silica_markdown::parse(&gone);
        let fs4 = foldables(&doc, &gone, FoldOptions::default());
        assert!(folds.resolve(&fs4).is_empty());
        assert!(folds.keys().is_empty());
    }

    #[test]
    fn a_record_restores_by_from_line_and_the_extent_is_re_derived() {
        let fs = all();
        // The reading view stored `{19,20}`: the editor folds 19..26.
        let mut folds = Folds::from_record(&fs, [19, 5]);
        assert_eq!(folds.resolve(&fs), vec![(20, 26)]);
        assert_eq!(folds.record(&fs), vec![(19, 26)]);
        assert!(is_hidden(&[(20, 26)], 23));
        assert!(!is_hidden(&[(20, 26)], 19));
        assert_eq!(hidden_end(&[(20, 26)], 23), Some(26));
        // A jump into the section unfolds it.
        assert!(folds.unfold_hiding(&fs, 21));
        assert!(folds.keys().is_empty());
    }

    #[test]
    fn the_options_turn_each_kind_off() {
        let doc = silica_markdown::parse(NOTE);
        let no_lists = foldables(
            &doc,
            NOTE,
            FoldOptions {
                headings: true,
                lists: false,
            },
        );
        assert!(no_lists.iter().all(|f| f.kind != FoldKind::List));
        let no_headings = foldables(
            &doc,
            NOTE,
            FoldOptions {
                headings: false,
                lists: true,
            },
        );
        assert_eq!(no_headings.len(), 1);
    }

    #[test]
    fn heading_keys_count_repeats() {
        let keys = heading_keys([(2, "A"), (2, "B"), (2, "A")].into_iter());
        assert_eq!(keys[0].nth, 0);
        assert_eq!(keys[2].nth, 1);
        assert_eq!(keys[1].kind, FoldKind::Heading(2));
    }

    #[test]
    fn the_store_round_trips_a_record_outside_the_vault() {
        let dir = tempfile::tempdir().expect("a temp dir");
        std::env::set_var("SILICA_CONFIG_DIR", dir.path());
        let root = dir.path().join("vault");
        let mut store = FoldStore::open(&root);
        store.put(
            "Sub Folder/Fold Probe.md",
            Record {
                folds: vec![FoldRange { from: 19, to: 26 }],
                lines: 33,
            },
        );
        let again = FoldStore::open(&root);
        assert_eq!(
            again
                .get("Sub Folder/Fold Probe.md")
                .map(|r| r.folds[0].from),
            Some(19)
        );
        // The file is under the configuration folder, not the vault.
        let files: Vec<PathBuf> = std::fs::read_dir(dir.path().join("note-fold"))
            .expect("the folder")
            .map(|e| e.expect("an entry").path())
            .collect();
        assert_eq!(files.len(), 1);
        assert!(!root.exists());
        std::env::remove_var("SILICA_CONFIG_DIR");
    }
}
