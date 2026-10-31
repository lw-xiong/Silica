//! The Writer gate's round trips: the vault comes back byte-identical.
//!
//! The gate for phase 3 is that a week of mixed use in Silica and Obsidian
//! leaves the vault's bytes as they were. These tests hold the half of that
//! which is ours to decide — every path from a keystroke to a file's bytes —
//! over the 1000-note conformance corpus and the `.obsidian` config sets in
//! the repository, so a change to the buffer, the edit plans, the properties
//! serializer or the config writer that starts touching bytes it did not
//! change fails here.
//!
//! Five properties, one per shape of writing:
//!
//! - **Opening and saving changes nothing.** A file loaded and written back
//!   is its own bytes, line endings and byte order mark included.
//! - **Every command is invertible.** A scripted editing session wound all
//!   the way back through undo leaves the original bytes.
//! - **An edit is local.** Typing changes the bytes at the cursor and
//!   nothing else; a property write stays inside the properties block, and
//!   an accepted autocompletion replaces the query it was offered for.
//! - **A config file read and written back is unchanged**, unknown keys
//!   included.
//! - **A canvas Obsidian saved is written back unchanged**: a `.canvas` in
//!   Obsidian's own layout comes back byte-identical from the canvas writer.
//! - **A base Obsidian saved is written back unchanged**: a `.base` in the
//!   canonical form comes back byte-identical from the base writer.
//!
//! The notes are the corpus; set `SILICA_ROUNDTRIP_VAULT` to a real vault's
//! path to run the same round trips over it as well (nothing is written to
//! it — every edit happens in a buffer in memory). That is how the gate's
//! "a week of mixed use" vault is checked without a test depending on one
//! being there.
//!
//! What is checked elsewhere: link rewriting on rename
//! (`silica-cache/tests/rename_corpus.rs`) and the properties planner itself
//! (`silica-cache/tests/properties_corpus.rs`). This harness drives the
//! writer's own path into those, not their internals.

use silica_cache::links::LinkFormat;
use silica_cache::{index_markdown, note_properties, LinkResolver, MetadataCache};
use silica_formats::config::{
    AppConfig, AppearanceConfig, CommandPaletteConfig, CommunityPlugins, CorePlugins,
    DailyNotesConfig, GraphConfig, Hotkeys, PropertyTypes, SwitcherConfig, TemplatesConfig,
};
use silica_formats::properties::{self as props, Change, Value};
use silica_text::{find, Cursors, EditKind, LinePrefix, LinkSyntax, Sel, TextBuffer};
use silica_vault::{Vault, VaultFile};
use std::ops::Range;
use std::path::{Path, PathBuf};

// ----- the corpus ----------------------------------------------------------

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("silica-markdown/tests/corpus/vault")
}

fn corpus_notes() -> Vec<PathBuf> {
    let root = corpus_root();
    let mut out = Vec::new();
    collect(&root, &mut out);
    out.sort();
    assert!(
        out.len() > 900,
        "corpus looks wrong: {} notes under {}",
        out.len(),
        root.display()
    );
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if !path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'))
            {
                collect(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
}

/// The notes under test: the corpus, and a real vault when
/// `SILICA_ROUNDTRIP_VAULT` names one.
fn notes_under_test() -> Vec<PathBuf> {
    let mut notes = corpus_notes();
    if let Some(root) = std::env::var_os("SILICA_ROUNDTRIP_VAULT") {
        let root = PathBuf::from(root);
        assert!(
            root.is_dir(),
            "SILICA_ROUNDTRIP_VAULT is not a directory: {}",
            root.display()
        );
        let before = notes.len();
        let mut extra = Vec::new();
        collect(&root, &mut extra);
        extra.sort();
        notes.extend(extra);
        assert!(notes.len() > before, "no notes under {}", root.display());
    }
    notes
}

/// The buffer's own invariants, checked after every step of a session: the
/// line index is the one the text implies, every cursor sits on a character
/// boundary inside the text, and what a save would write reads back as the
/// text in hand.
fn check_invariants(buf: &TextBuffer, note: &Path, step: &str) {
    let where_ = || format!("{} after {step}", note.display());
    let starts: Vec<usize> = std::iter::once(0)
        .chain(buf.text().match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    assert_eq!(
        buf.line_starts(),
        starts.as_slice(),
        "line index: {}",
        where_()
    );
    for sel in buf.cursors().sels {
        for at in [sel.anchor, sel.head] {
            assert!(at <= buf.len(), "cursor past the end: {}", where_());
            assert!(
                buf.text().is_char_boundary(at),
                "cursor off a character boundary: {}",
                where_()
            );
        }
    }
    let saved = buf.file_contents();
    assert!(
        buf.matches_file(&saved),
        "a save would not read back as the buffer: {}",
        where_()
    );
}

/// Byte offsets spread through a note, each on a character boundary: the
/// start, three points inside and the end.
fn probe_offsets(buf: &TextBuffer) -> Vec<usize> {
    let len = buf.len();
    let mut out: Vec<usize> = [0, len / 4, len / 2, (len * 3) / 4, len]
        .iter()
        .map(|&at| buf.clamp_offset(at))
        .collect();
    out.dedup();
    out
}

/// One scripted editing session at `at`: the keys that write bytes, in the
/// order a person might press them. Every step goes through the same
/// `TextBuffer` methods the editor's actions call.
fn drive_session(buf: &mut TextBuffer, at: usize, note: &Path) {
    let step = |buf: &TextBuffer, name: &str| check_invariants(buf, note, name);

    buf.set_cursor(at, None);
    buf.insert_all("note", EditKind::Typing);
    step(buf, "typing");
    buf.newline(4, true);
    step(buf, "Enter");
    buf.insert_all("- item", EditKind::Typing);
    step(buf, "typing a list item");
    buf.indent(true, 4, true);
    step(buf, "Tab");
    buf.outdent(true, 4, true);
    step(buf, "Shift+Tab");
    buf.newline(4, true);
    step(buf, "Enter on an item");
    buf.backspace();
    step(buf, "Backspace");
    buf.delete_forward();
    step(buf, "Delete");
    buf.delete_word_left();
    step(buf, "Ctrl+Backspace");
    buf.paste("pasted\nover two lines");
    step(buf, "paste");
    // A selection replaced by typing, then the cursor's line cut.
    let head = buf.head;
    buf.set_cursor(head, Some(head.saturating_sub(12)));
    buf.insert_all("x", EditKind::Typing);
    step(buf, "typing over a selection");
    let ranges = buf.copy_ranges();
    buf.edit_many(
        ranges.into_iter().map(|r| (r, String::new())).collect(),
        0,
        None,
    );
    step(buf, "cutting the line");
    // The formatting commands, over whatever the session has left under
    // the cursor.
    buf.toggle_wrap("**");
    step(buf, "Ctrl+B");
    buf.toggle_checklist();
    step(buf, "Ctrl+L");
    buf.set_heading(2);
    step(buf, "Set as heading 2");
    buf.set_heading(0);
    step(buf, "Remove heading");
    buf.toggle_line_prefix(LinePrefix::Bullet);
    step(buf, "Toggle bullet list");
    buf.toggle_line_prefix(LinePrefix::Ordered);
    step(buf, "Toggle numbered list");
    buf.toggle_line_prefix(LinePrefix::Quote);
    step(buf, "Toggle blockquote");
    buf.insert_link(LinkSyntax::Wikilink);
    step(buf, "Add internal link");
    buf.swap_lines(true);
    step(buf, "Move line up");
    buf.swap_lines(false);
    step(buf, "Move line down");
    buf.delete_paragraph();
    step(buf, "Delete paragraph");
    // Deleting the paragraph can empty a note that was empty to start
    // with, and the session has to have written something.
    buf.insert_all("tail", EditKind::Typing);
    step(buf, "typing after the deletion");
}

/// Winds the whole history back and returns what a save would write.
fn undo_all(buf: &mut TextBuffer, note: &Path) -> String {
    let mut steps = 0;
    while buf.can_undo() {
        buf.undo();
        steps += 1;
        check_invariants(buf, note, "undo");
        assert!(steps < 1000, "undo does not terminate: {}", note.display());
    }
    buf.file_contents()
}

// ----- opening and saving --------------------------------------------------

#[test]
fn opening_every_note_and_saving_it_changes_no_bytes() {
    let notes = notes_under_test();
    let mut with_bom = 0;
    let mut with_crlf = 0;
    for note in &notes {
        let raw = std::fs::read_to_string(note).unwrap();
        let buf = TextBuffer::from_file(&raw);
        assert_eq!(
            buf.file_contents(),
            raw,
            "opening and saving changed {}",
            note.display()
        );
        assert!(
            !buf.is_dirty(),
            "a fresh buffer is dirty: {}",
            note.display()
        );
        assert!(
            !buf.text().contains('\r'),
            "a carriage return reached the buffer: {}",
            note.display()
        );
        with_bom += usize::from(buf.bom());
        with_crlf += usize::from(buf.crlf());
        check_invariants(&buf, note, "open");
    }
    // The corpus has to keep covering both, or this test proves nothing
    // about them.
    assert!(with_bom > 0, "no note in the corpus has a byte order mark");
    assert!(with_crlf > 0, "no note in the corpus uses CRLF");
}

// ----- every command is invertible -----------------------------------------

#[test]
fn every_editing_command_undoes_back_to_the_original_bytes() {
    for note in notes_under_test() {
        let raw = std::fs::read_to_string(&note).unwrap();
        for at in probe_offsets(&TextBuffer::from_file(&raw)) {
            let mut buf = TextBuffer::from_file(&raw);
            drive_session(&mut buf, at, &note);
            assert_ne!(
                buf.file_contents(),
                raw,
                "the session wrote nothing in {} at {at}",
                note.display()
            );
            assert!(
                buf.is_dirty(),
                "an edited buffer is clean: {}",
                note.display()
            );
            assert_eq!(
                undo_all(&mut buf, &note),
                raw,
                "undo did not restore {} after a session at {at}",
                note.display()
            );
        }
    }
}

#[test]
fn an_edit_at_several_cursors_undoes_back_to_the_original_bytes() {
    for note in notes_under_test() {
        let raw = std::fs::read_to_string(&note).unwrap();
        let mut buf = TextBuffer::from_file(&raw);
        let offsets = probe_offsets(&buf);
        if offsets.len() < 3 {
            continue;
        }
        buf.set_cursors(Cursors {
            sels: offsets.iter().map(|&at| Sel::at(at)).collect(),
            primary: 0,
        });
        let cursors = buf.cursor_count();
        buf.insert_all("multi", EditKind::Typing);
        check_invariants(&buf, &note, "typing at several cursors");
        assert_eq!(
            buf.cursor_count(),
            cursors,
            "a cursor was lost typing in {}",
            note.display()
        );
        assert_eq!(
            buf.text().matches("multi").count() - raw.matches("multi").count(),
            cursors,
            "not every cursor typed in {}",
            note.display()
        );
        buf.paste("one\ntwo\nthree\nfour\nfive");
        check_invariants(&buf, &note, "pasting at several cursors");
        assert_eq!(
            undo_all(&mut buf, &note),
            raw,
            "undo did not restore {} after a multi-cursor edit",
            note.display()
        );
    }
}

// ----- an edit is local ----------------------------------------------------

#[test]
fn typing_changes_only_the_bytes_at_the_cursor() {
    for note in notes_under_test() {
        let raw = std::fs::read_to_string(&note).unwrap();
        let mut buf = TextBuffer::from_file(&raw);
        let before = buf.text().to_string();
        for at in probe_offsets(&buf) {
            buf.set_cursor(at, None);
            buf.insert_all("typed", EditKind::Typing);
            let want = format!("{}typed{}", &before[..at], &before[at..]);
            assert_eq!(
                buf.text(),
                want,
                "typing at {at} moved other bytes in {}",
                note.display()
            );
            buf.undo();
            assert_eq!(
                buf.text(),
                before,
                "undo of typing at {at} in {}",
                note.display()
            );
        }
    }
}

/// A vault for the suggester to offer rows from: two notes that exist, so
/// `[[al` has something to resolve to whatever note is under test.
fn suggest_fixture() -> (Vec<VaultFile>, MetadataCache) {
    let notes = [
        (
            "Alpha.md",
            "# Alpha\n\n## First Heading\n\nA paragraph. ^blockone\n",
        ),
        ("Beta Note.md", "# Beta Note\n"),
    ];
    let mut resolver = LinkResolver::default();
    for (path, _) in notes {
        resolver.insert(path);
    }
    let files = notes
        .iter()
        .map(|(path, _)| VaultFile {
            path: (*path).to_string(),
            kind: silica_formats::FileKind::from_path(path),
            size: 0,
            mtime: 0,
            ctime: 0,
        })
        .collect();
    let cache = MetadataCache::new(
        resolver,
        notes.iter().map(|(path, src)| (*path, index_markdown(src))),
    );
    (files, cache)
}

#[test]
fn accepting_a_suggestion_replaces_only_the_query_and_undoes_cleanly() {
    let (files, cache) = suggest_fixture();
    let opts = silica_suggest::Options {
        format: LinkFormat::Shortest,
        markdown_links: false,
    };
    for note in notes_under_test() {
        let raw = std::fs::read_to_string(&note).unwrap();
        for at in probe_offsets(&TextBuffer::from_file(&raw)) {
            let mut buf = TextBuffer::from_file(&raw);
            let before = buf.text().to_string();
            // Type the query the way the editor's keystrokes do.
            buf.set_cursor(at, None);
            buf.insert_all("[[al", EditKind::Typing);
            check_invariants(&buf, &note, "suggest typing");
            let cursor = buf.cursor();
            let line = buf.line_text(cursor.line).to_string();
            let t = silica_suggest::trigger(&line, cursor.col)
                .expect("a `[[` query triggers wherever it is typed");
            let vault = silica_suggest::Vault {
                files: &files,
                cache: &cache,
                source: "Note.md",
                show_unsupported: false,
                text: &|_| None,
            };
            let items = silica_suggest::suggestions(&t, &vault);
            let item = &items.first().expect("`al` offers Alpha").item;
            let ins =
                silica_suggest::insert(&t, item, silica_suggest::Accept::Whole, &vault, &opts);
            let line_start = buf.line_range(cursor.line).start;
            let range = line_start + t.range.start..line_start + t.range.end + ins.consume;
            let cursor_to = range.start + ins.cursor;
            assert!(
                buf.edit_many(vec![(range.clone(), ins.text.clone())], cursor_to, None),
                "accepting wrote nothing in {} at {at}",
                note.display()
            );
            check_invariants(&buf, &note, "suggest accept");
            // Only the query is replaced: the bytes before it and the bytes
            // after what it consumed are the note's own.
            let want = format!(
                "{}{}{}",
                &before[..range.start],
                ins.text,
                &before[at + ins.consume..]
            );
            assert_eq!(
                buf.text(),
                want,
                "accepting at {at} moved other bytes in {}",
                note.display()
            );
            assert_eq!(
                undo_all(&mut buf, &note),
                raw,
                "undo did not restore {} after accepting at {at}",
                note.display()
            );
        }
    }
}

#[test]
fn writing_a_block_id_touches_only_that_block_and_reindexes_onto_it() {
    use silica_suggest::blocks::{new_id, plan_id, Placement};
    let (mut checked, mut refused) = (0, 0);
    for note in notes_under_test() {
        let raw = std::fs::read_to_string(&note).unwrap();
        let buf = TextBuffer::from_file(&raw);
        // The planning is over the file's own bytes, endings and all.
        let text = raw.as_str();
        let nl = if buf.crlf() { "\r\n" } else { "\n" };
        let meta = index_markdown(buf.text());
        // Three blocks per note is enough to cover the shapes without
        // making the corpus pass take minutes.
        let blocks: Vec<_> = meta
            .sections
            .iter()
            .filter(|s| s.id.is_none())
            .filter_map(|s| Placement::of(s.kind).map(|p| (s.position.end.offset, p)))
            .take(3)
            .collect();
        for (end, placement) in blocks {
            // The cache's offsets are into the buffer's text, which has had
            // a BOM and CRLF taken off; map back onto the raw bytes.
            let Some(end) = raw_offset(&raw, buf.text(), end) else {
                continue;
            };
            let id = new_id(end as u64, &meta.blocks);
            // A block an id cannot be written onto is refused rather than
            // written to: an unclosed code fence runs to the end of the
            // file, and an id after it would land inside the fence.
            let Some(ins) = plan_id(text, end, placement, &id, nl) else {
                refused += 1;
                continue;
            };
            let after = ins.applied(text);
            checked += 1;
            // Only the insertion changed: everything before it and
            // everything after what it replaced is the note's own bytes.
            assert_eq!(
                &after[..ins.range.start],
                &text[..ins.range.start],
                "bytes before the id moved in {}",
                note.display()
            );
            assert_eq!(
                &after[ins.range.start + ins.text.len()..],
                &text[ins.range.end..],
                "bytes after the id moved in {}",
                note.display()
            );
            // The id reads back onto a block, which is what makes the link
            // Silica just wrote resolve rather than dangle.
            let reindexed = index_markdown(TextBuffer::from_file(&after).text());
            assert!(
                reindexed.blocks.contains_key(&id),
                "the id did not re-index onto a block in {} ({placement:?})",
                note.display()
            );
            // A note is only ever written with its own endings.
            if buf.crlf() {
                assert!(
                    !ins.text.contains('\n') || ins.text.contains("\r\n"),
                    "a bare newline reached a CRLF note: {}",
                    note.display()
                );
            }
        }
    }
    assert!(checked > 100, "only {checked} blocks were written to");
    // The refusals are the rare shape, not the common one.
    assert!(
        refused * 20 < checked,
        "{refused} blocks of {checked} refused an id"
    );
}

/// The offset in `raw` of the byte at `at` in `text`, where `text` is `raw`
/// with a byte order mark and carriage returns taken off.
fn raw_offset(raw: &str, text: &str, at: usize) -> Option<usize> {
    if raw.len() == text.len() {
        return (at <= raw.len()).then_some(at);
    }
    // Walk both, skipping what the buffer dropped.
    let (mut r, mut t) = (0usize, 0usize);
    let rb = raw.as_bytes();
    while t < at && r < rb.len() {
        if rb[r] == b'\r' && rb.get(r + 1) == Some(&b'\n') {
            r += 1;
            continue;
        }
        if r == 0 && raw.starts_with('\u{feff}') {
            r += '\u{feff}'.len_utf8();
            continue;
        }
        r += 1;
        t += 1;
    }
    (t == at).then_some(r)
}

#[test]
fn inserting_a_template_writes_only_its_own_bytes_and_undoes_cleanly() {
    use silica_templates::{expand, Context, Stamp, DEFAULT_DATE_FORMAT, DEFAULT_TIME_FORMAT};
    let at = Stamp {
        year: 2026,
        month: 10,
        day: 18,
        hour: 17,
        minute: 53,
        second: 9,
    };
    // A template with every placeholder, its own CRLF endings and a byte
    // order mark: only its text may reach the note.
    let raw_template = "\u{feff}# {{title}}\r\n\r\nWritten {{date}} at {{time}}.\r\n";
    let template = TextBuffer::from_file(raw_template).text().to_string();
    for note in notes_under_test() {
        let raw = std::fs::read_to_string(&note).unwrap();
        let mut buf = TextBuffer::from_file(&raw);
        let before = buf.text().to_string();
        for at_offset in probe_offsets(&buf) {
            let cx = Context {
                at,
                date_format: DEFAULT_DATE_FORMAT,
                time_format: DEFAULT_TIME_FORMAT,
                title: "Note",
            };
            let text = expand(&template, &cx);
            assert!(
                !text.contains('\r') && !text.contains('\u{feff}'),
                "a template's own endings reached the note"
            );
            buf.set_cursor(at_offset, None);
            assert!(buf.insert_at_cursors(&text));
            check_invariants(&buf, &note, "template insertion");
            let want = format!("{}{}{}", &before[..at_offset], text, &before[at_offset..]);
            assert_eq!(
                buf.text(),
                want,
                "inserting at {at_offset} moved other bytes in {}",
                note.display()
            );
            buf.undo();
            assert_eq!(
                buf.text(),
                before,
                "undo of a template insertion at {at_offset} in {}",
                note.display()
            );
        }
        assert_eq!(
            buf.file_contents(),
            raw,
            "{} did not round trip",
            note.display()
        );
    }
}

#[test]
fn a_table_command_stays_inside_its_table_and_undoes_cleanly() {
    use silica_text::table::{tables, TableCommand};
    const COMMANDS: &[TableCommand] = &[
        TableCommand::RowBefore,
        TableCommand::RowAfter,
        TableCommand::RowCopy,
        TableCommand::RowDelete,
        TableCommand::ColBefore,
        TableCommand::ColAfter,
        TableCommand::ColCopy,
        TableCommand::AlignCenter,
    ];
    let mut count = 0;
    for note in notes_under_test() {
        let raw = std::fs::read_to_string(&note).unwrap();
        let text = TextBuffer::from_file(&raw).text().to_string();
        // Every table in the note, through the cursor in its first cell.
        for found in tables(&text) {
            count += 1;
            for cmd in COMMANDS {
                let mut buf = TextBuffer::from_file(&raw);
                buf.set_cursor(found.start + 2, None);
                if !buf.table_command(*cmd) {
                    continue;
                }
                check_invariants(&buf, &note, "table command");
                // A table command rewrites its whole table, and Obsidian's
                // does too — but nothing outside it may move.
                let after = buf.text().to_string();
                assert_eq!(
                    &after[..found.start],
                    &text[..found.start],
                    "{cmd:?} moved bytes before the table in {}",
                    note.display()
                );
                assert_eq!(
                    &after[after.len() - (text.len() - found.end)..],
                    &text[found.end..],
                    "{cmd:?} moved bytes after the table in {}",
                    note.display()
                );
                assert_eq!(
                    undo_all(&mut buf, &note),
                    raw,
                    "undo did not restore {} after {cmd:?}",
                    note.display()
                );
            }
        }
    }
    assert!(count > 0, "no note under test has a table");
}

#[test]
fn a_save_refuses_a_file_that_changed_underneath() {
    // The gate is a week of mixed use in both apps, and this is the shape
    // that loses data: a note is open and edited here, the other app saves
    // it, and the autosave writes the whole buffer over that save. The
    // buffer keeps a fingerprint of the bytes it last knew on disk so the
    // save can refuse.
    for note in notes_under_test() {
        let raw = std::fs::read_to_string(&note).unwrap();
        let mut buf = TextBuffer::from_file(&raw);
        // Nothing has happened: the file on disk is the file we loaded.
        assert!(
            buf.disk_unchanged(&raw),
            "a freshly loaded note did not match its own bytes: {}",
            note.display()
        );
        // Our own edits do not change what is on disk.
        buf.set_cursor(0, None);
        buf.insert_all("typed", EditKind::Typing);
        assert!(
            buf.disk_unchanged(&raw),
            "editing changed what the buffer thinks is on disk: {}",
            note.display()
        );
        // The other app saves: one byte is enough to notice.
        let theirs = format!("{raw}\nwritten by the other app\n");
        assert!(
            !buf.disk_unchanged(&theirs),
            "a change underneath went unnoticed in {}",
            note.display()
        );
        // Once we save, what we wrote is what is on disk.
        let ours = buf.file_contents();
        buf.mark_saved();
        buf.mark_on_disk(&ours);
        assert!(buf.disk_unchanged(&ours));
        assert!(!buf.disk_unchanged(&raw), "the pre-save bytes still match");
    }
}

#[test]
fn an_edited_file_keeps_its_line_ending_and_byte_order_mark() {
    // The one corpus note written the Windows way, with both.
    let note = corpus_root().join("edge/CRLF.md");
    let raw = std::fs::read_to_string(&note).unwrap();
    let mut buf = TextBuffer::from_file(&raw);
    assert!(
        buf.bom() && buf.crlf(),
        "the CRLF probe note lost its shape"
    );

    let tail = buf.len();
    buf.set_cursor(tail, None);
    buf.insert_all("tail", EditKind::Typing);
    buf.newline(4, true);
    let saved = buf.file_contents();
    assert!(
        saved.starts_with('\u{feff}'),
        "the byte order mark was dropped"
    );
    assert_eq!(
        saved.matches('\n').count(),
        saved.matches("\r\n").count(),
        "an edit left a bare newline in a CRLF file"
    );
    assert!(
        saved.starts_with(&raw),
        "an edit at the end rewrote earlier bytes"
    );
    assert_eq!(
        undo_all(&mut buf, &note),
        raw,
        "undo did not restore the CRLF note"
    );
}

// ----- properties ----------------------------------------------------------

/// The editor's property commit path: plan the changes against the block and
/// apply them as one undo step (`Editor::apply_property_changes`).
fn commit_properties(buf: &mut TextBuffer, changes: &[Change]) -> bool {
    let Some(block) = note_properties(buf.text()) else {
        return false;
    };
    let Some(edits) = props::plan(buf.text(), &block, changes) else {
        return false;
    };
    let ranges: Vec<(Range<usize>, String)> = edits
        .into_iter()
        .map(|e| (e.start..e.end, e.text))
        .collect();
    if ranges.is_empty() {
        return false;
    }
    let head = buf.head.min(buf.len());
    buf.apply_edits(ranges, Cursors::at(head), EditKind::Other);
    true
}

#[test]
fn a_property_write_touches_only_its_block_and_undoes_cleanly() {
    let mut written = 0usize;
    for note in notes_under_test() {
        let raw = std::fs::read_to_string(&note).unwrap();
        let mut buf = TextBuffer::from_file(&raw);
        let Some(block) = note_properties(buf.text()) else {
            continue;
        };
        let body_before = buf.text()[block.block.end..].to_string();
        let change = Change::Set {
            key: "silica-roundtrip".into(),
            value: Value::Text("probe".into()),
        };
        if !commit_properties(&mut buf, &[change]) {
            continue;
        }
        written += 1;
        check_invariants(&buf, &note, "a property write");
        let after = note_properties(buf.text())
            .unwrap_or_else(|| panic!("the block vanished from {}", note.display()));
        assert_eq!(
            buf.text()[after.block.end..],
            body_before,
            "a property write changed the body of {}",
            note.display()
        );
        assert!(
            after.get("silica-roundtrip").is_some(),
            "the property was not written to {}",
            note.display()
        );
        assert_eq!(
            undo_all(&mut buf, &note),
            raw,
            "undo did not restore {} after a property write",
            note.display()
        );
    }
    assert!(
        written > 900,
        "only {written} corpus notes took a property write"
    );
}

// ----- find and replace ----------------------------------------------------

#[test]
fn replacing_every_match_and_undoing_restores_the_note() {
    let opts = find::Options::default();
    let re = find::compile("the", opts).unwrap();
    let mut replaced = 0usize;
    for note in notes_under_test() {
        let raw = std::fs::read_to_string(&note).unwrap();
        let mut buf = TextBuffer::from_file(&raw);
        let matches = find::find_matches(buf.text(), "the", opts);
        if matches.is_empty() {
            continue;
        }
        replaced += 1;
        let edits: Vec<(Range<usize>, String)> = matches
            .iter()
            .map(|m| {
                let with = find::replacement_for(buf.text(), m, &re, opts, "THE");
                (m.clone(), with)
            })
            .collect();
        let last_end = edits.last().map(|(r, _)| r.end).unwrap_or(0);
        let cursor = silica_text::shifted(last_end, &edits);
        let len = buf.len();
        assert!(buf.edit_many(edits, cursor, None));
        check_invariants(&buf, &note, "replace all");
        assert_eq!(
            buf.len(),
            len,
            "a same-length replacement changed the size of {}",
            note.display()
        );
        assert_eq!(
            undo_all(&mut buf, &note),
            raw,
            "undo did not restore {} after replacing every match",
            note.display()
        );
    }
    assert!(replaced > 500, "only {replaced} corpus notes had a match");
}

// ----- the config files ----------------------------------------------------

/// The `.obsidian` sets in the repository: the corpus vault's, and the docs
/// vault Silica dogfoods. Both were written by Obsidian itself.
fn config_dirs() -> Vec<PathBuf> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut dirs = vec![
        corpus_root().join(".obsidian"),
        repo.parent().unwrap().join("docs/silica/.obsidian"),
    ];
    // The gate's real vault, when one is named: its `.obsidian` is the set a
    // person actually accumulated, which the repository's two cannot imitate
    // — files missing outright, an `appearance.json` that is a bare `{}`, no
    // trailing newline. A tree that is not a vault root simply has none.
    if let Some(root) = std::env::var_os("SILICA_ROUNDTRIP_VAULT") {
        let dir = PathBuf::from(root).join(".obsidian");
        if dir.is_dir() {
            dirs.push(dir);
        }
    }
    dirs
}

/// A vault whose `.obsidian` is a copy of `dir`, so a write never touches
/// the repository.
fn vault_with_config(dir: &Path) -> (tempfile::TempDir, Vault) {
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join(".obsidian");
    std::fs::create_dir_all(&dest).unwrap();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        if entry.path().is_file() {
            std::fs::copy(entry.path(), dest.join(entry.file_name())).unwrap();
        }
    }
    let vault = Vault::open(tmp.path()).unwrap();
    (tmp, vault)
}

/// Reads `name` as `T`, writes it straight back and compares the bytes.
/// Obsidian rewrites a config file whole on every change, so what Silica
/// writes has to be what Obsidian wrote, down to the indentation and the
/// keys Silica does not model (`#[serde(flatten)] extra`).
fn config_round_trips<T>(vault: &Vault, dir: &Path, name: &str) -> bool
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    let path = dir.join(name);
    let Ok(before) = std::fs::read_to_string(&path) else {
        return false;
    };
    let value: T = vault
        .read_config(name)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
        .unwrap_or_else(|| panic!("{} did not parse", path.display()));
    vault.write_config(name, &value).unwrap();
    let after = std::fs::read_to_string(vault.root().join(".obsidian").join(name)).unwrap();
    assert_eq!(
        after,
        before,
        "writing {} back changed its bytes",
        path.display()
    );
    true
}

#[test]
fn every_config_file_is_written_back_byte_identical() {
    let mut checked = 0usize;
    for dir in config_dirs() {
        assert!(dir.is_dir(), "missing config set: {}", dir.display());
        let (_tmp, vault) = vault_with_config(&dir);
        let before = checked;
        checked += usize::from(config_round_trips::<AppConfig>(&vault, &dir, "app.json"));
        checked += usize::from(config_round_trips::<AppearanceConfig>(
            &vault,
            &dir,
            "appearance.json",
        ));
        checked += usize::from(config_round_trips::<CorePlugins>(
            &vault,
            &dir,
            "core-plugins.json",
        ));
        checked += usize::from(config_round_trips::<CommunityPlugins>(
            &vault,
            &dir,
            "community-plugins.json",
        ));
        checked += usize::from(config_round_trips::<GraphConfig>(
            &vault,
            &dir,
            "graph.json",
        ));
        checked += usize::from(config_round_trips::<Hotkeys>(&vault, &dir, "hotkeys.json"));
        checked += usize::from(config_round_trips::<PropertyTypes>(
            &vault,
            &dir,
            "types.json",
        ));
        // The per-plugin files. Obsidian does not create any of them until a
        // setting in that plugin is written, so no set here has one yet and
        // these contribute nothing — they are listed so a set that does have
        // one covers it, `switcher.json` above all: the app writes it.
        checked += usize::from(config_round_trips::<SwitcherConfig>(
            &vault,
            &dir,
            "switcher.json",
        ));
        checked += usize::from(config_round_trips::<CommandPaletteConfig>(
            &vault,
            &dir,
            "command-palette.json",
        ));
        checked += usize::from(config_round_trips::<TemplatesConfig>(
            &vault,
            &dir,
            "templates.json",
        ));
        checked += usize::from(config_round_trips::<DailyNotesConfig>(
            &vault,
            &dir,
            "daily-notes.json",
        ));
        // Every set has to contribute, or naming a vault proves nothing
        // about its own `.obsidian`.
        assert!(
            checked > before,
            "no config file round-tripped in {}",
            dir.display()
        );
    }
    // `workspace.json` is deliberately not here: Silica neither models nor
    // writes it (`docs/silica/.obsidian` keeps it out of version control for
    // the same reason).
    //
    // The two repository sets hold eight files between them. No set in the
    // repository has a `hotkeys.json` or a `types.json`, so those two writers
    // are only covered when a vault that has them is named.
    assert!(
        checked >= 8,
        "only {checked} config files were round-tripped"
    );
}

// ----- canvases --------------------------------------------------------------

/// Every `.canvas` under `dir`, dot-folders skipped.
fn collect_canvases(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if !path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'))
            {
                collect_canvases(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "canvas") {
            out.push(path);
        }
    }
}

/// Whether a `:` is followed by a space outside a string literal. Obsidian's
/// writer puts no space after a colon; every pretty-printer does. Scanning
/// rather than searching for `": "` keeps a label like `"Area: north"` out of
/// the answer.
fn padded_colon_outside_a_string(text: &str) -> bool {
    let bytes = text.as_bytes();
    let (mut in_string, mut escaped) = (false, false);
    for (i, &b) in bytes.iter().enumerate() {
        if in_string {
            match b {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
        } else if b == b'"' {
            in_string = true;
        } else if b == b':' && bytes.get(i + 1) == Some(&b' ') {
            return true;
        }
    }
    false
}

/// Obsidian's own layout: a tab-indented object with no trailing newline and
/// no padding after a colon. A canvas authored by anything else is rewritten
/// whole the first time Obsidian saves it, so there is nothing byte-level to
/// hold the writer to until then.
///
/// The first and last bytes alone do not separate the two, since a
/// pretty-printer also opens `{\n\t` and can end without a newline. Nor does
/// one-object-per-line: Obsidian expands an element that *nests* an object
/// over several lines (`scrambled.out.canvas` has one). The colon is what
/// tells them apart.
fn in_obsidians_layout(text: &str) -> bool {
    text.starts_with("{\n\t") && !text.ends_with('\n') && !padded_colon_outside_a_string(text)
}

/// The canvases Obsidian wrote back for files it had read in the CDP probe
/// runs are always here (what it writes for a canvas made in the app, or on
/// closing one unedited, is not a fixed point: see
/// `silica-formats/tests/canvas_obsidian.rs`); a real vault's own are added
/// when `SILICA_ROUNDTRIP_VAULT` names one.
#[test]
fn every_canvas_obsidian_saved_is_written_back_byte_identical() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let fixtures = repo.join("silica-formats/tests/canvas");
    let mut canvases: Vec<PathBuf> = ["scrambled", "order", "noedges"]
        .iter()
        .map(|n| fixtures.join(format!("{n}.out.canvas")))
        .collect();
    if let Some(root) = std::env::var_os("SILICA_ROUNDTRIP_VAULT") {
        let mut extra = Vec::new();
        collect_canvases(Path::new(&root), &mut extra);
        extra.sort();
        canvases.extend(extra);
    }
    let (mut checked, mut authored_elsewhere, mut empty) = (0, Vec::new(), Vec::new());
    for path in &canvases {
        let before = std::fs::read_to_string(path).unwrap();
        // `canvas:new-file` writes a zero-byte file, and Obsidian puts JSON
        // in it only once the canvas holds something (see
        // [[Specification#Observed canvas write behavior]]), so an untouched
        // new canvas is a real vault state with nothing to round trip. It is
        // skipped rather than failed: a vault that holds one is not a vault
        // the writer has mangled.
        if before.trim().is_empty() {
            empty.push(path.display().to_string());
            continue;
        }
        let canvas = silica_formats::canvas::Canvas::from_json(&before)
            .unwrap_or_else(|e| panic!("{} did not parse: {e}", path.display()));
        if !in_obsidians_layout(&before) {
            authored_elsewhere.push(path.display().to_string());
            continue;
        }
        assert_eq!(
            canvas.to_json(),
            before,
            "writing {} back changed its bytes",
            path.display()
        );
        checked += 1;
    }
    if !authored_elsewhere.is_empty() {
        eprintln!(
            "{} canvases not in Obsidian's layout, which Obsidian itself would rewrite: {}",
            authored_elsewhere.len(),
            authored_elsewhere.join(", ")
        );
    }
    if !empty.is_empty() {
        eprintln!(
            "{} empty canvases, which is what a new one is until its first edit: {}",
            empty.len(),
            empty.join(", ")
        );
    }
    assert!(checked >= 3, "only {checked} canvases were round-tripped");
}

/// Obsidian's canonical form for a `.base`, told apart from a hand-written one
/// without asking the writer — which would make this test agree with itself.
///
/// A base is canonicalized rather than preserved
/// ([[Specification#Observed Bases write behavior, Obsidian 1.14.5 (installer build 1.13.7)]]),
/// so everything a person types that Obsidian strips is a tell: a comment, a
/// CRLF, a byte order mark, a flow collection, a missing or doubled trailing
/// newline, or an indent that is not a multiple of two.
fn in_obsidians_base_form(text: &str) -> bool {
    if text.is_empty() || text.starts_with('\u{feff}') || text.contains('\r') {
        return false;
    }
    if !text.ends_with('\n') || text.ends_with("\n\n") {
        return false;
    }
    text.lines().all(|line| {
        let indent = line.len() - line.trim_start().len();
        let body = line.trim_start();
        indent % 2 == 0 && !body.starts_with('#') && !body.contains(": [") && !body.contains(": {")
    })
}

/// The bases Obsidian wrote back in the CDP probe run are always here; a real
/// vault's own are added when `SILICA_ROUNDTRIP_VAULT` names one. No vault on
/// this machine has a `.base` yet, so today the fixtures are the whole corpus
/// — which is why the floor below is what it is.
#[test]
fn every_base_obsidian_saved_is_written_back_byte_identical() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let fixtures = repo.join("silica-formats/tests/base");
    let mut bases: Vec<PathBuf> = [
        "scrambled",
        "minimal",
        "layouts",
        "edited",
        "default-view",
        "broken-filter",
        "broken-formula",
        "broken-summary",
        "layout-change",
    ]
    .iter()
    .map(|n| fixtures.join(format!("{n}.out.base")))
    .collect();
    if let Some(root) = std::env::var_os("SILICA_ROUNDTRIP_VAULT") {
        let mut extra = Vec::new();
        collect_bases(Path::new(&root), &mut extra);
        extra.sort();
        bases.extend(extra);
    }
    let (mut checked, mut authored_elsewhere, mut empty) = (0, Vec::new(), Vec::new());
    for path in &bases {
        let before = std::fs::read_to_string(path).unwrap();
        // `bases:new-file` writes a zero-byte file, and Obsidian puts YAML in
        // it only once the base is opened and given its default view, so an
        // untouched new base is a real vault state with nothing to round trip.
        if before.trim().is_empty() {
            empty.push(path.display().to_string());
            continue;
        }
        let base = silica_formats::base::Base::from_yaml(&before)
            .unwrap_or_else(|e| panic!("{} did not parse: {e}", path.display()));
        if !in_obsidians_base_form(&before) {
            authored_elsewhere.push(path.display().to_string());
            continue;
        }
        assert_eq!(
            base.to_yaml(),
            before,
            "writing {} back changed its bytes",
            path.display()
        );
        checked += 1;
    }
    if !authored_elsewhere.is_empty() {
        eprintln!(
            "{} bases not in Obsidian's form, which Obsidian itself would rewrite: {}",
            authored_elsewhere.len(),
            authored_elsewhere.join(", ")
        );
    }
    if !empty.is_empty() {
        eprintln!(
            "{} empty bases, which is what a new one is until it is opened: {}",
            empty.len(),
            empty.join(", ")
        );
    }
    assert!(checked >= 9, "only {checked} bases were round-tripped");
}

fn collect_bases(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if !path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'))
            {
                collect_bases(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "base") {
            out.push(path);
        }
    }
}
