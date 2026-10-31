//! Properties against the conformance corpus: a ratchet that Silica never
//! rewrites a properties block it did not change.
//!
//! Obsidian rewrites the whole block on any property write — a `processFrontMatter`
//! call that mutates nothing still deletes comments and reflows lists (observed
//! against 1.14.4). The Writer gate is the opposite: the vault round-trips
//! byte-identical. These tests hold that over the 1000-note corpus plus the
//! hand-written edge cases, so a change to the parser, the scanner or the
//! serializer that starts touching bytes fails here.

use silica_cache::note_properties;
use silica_formats::properties::{apply, plan, Change, Value};
use std::path::{Path, PathBuf};

fn corpus_notes() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("silica-markdown/tests/corpus/vault");
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

/// Writing a property back as the value it already holds must plan no edit at
/// all. This is the property that keeps comments, flow sequences, quoting and
/// blank lines intact: an untouched entry is never re-serialized.
#[test]
fn rewriting_every_value_with_itself_changes_no_bytes() {
    let mut checked = 0usize;
    let mut entries = 0usize;
    for path in corpus_notes() {
        let text = std::fs::read_to_string(&path).unwrap();
        let Some(props) = note_properties(&text) else {
            continue;
        };
        checked += 1;
        for entry in &props.entries {
            entries += 1;
            let change = Change::Set {
                key: entry.key.clone(),
                value: entry.value.clone(),
            };
            if let Some(edits) = plan(&text, &props, std::slice::from_ref(&change)) {
                let after = apply(&text, &edits);
                assert_eq!(
                    after,
                    text,
                    "{}: rewriting `{}` with its own value changed the file",
                    path.display(),
                    entry.key
                );
            }
        }
    }
    assert!(checked > 900, "only {checked} notes had properties");
    assert!(entries > 2000, "only {entries} properties seen");
}

/// Every note's block parses, and every entry's span really covers the key.
#[test]
fn every_entry_span_covers_its_own_key() {
    for path in corpus_notes() {
        let text = std::fs::read_to_string(&path).unwrap();
        let Some(props) = note_properties(&text) else {
            continue;
        };
        for entry in &props.entries {
            let span = &text[entry.span.clone()];
            let key = &text[entry.key_span.clone()];
            assert!(
                entry.span.start <= entry.key_span.start && entry.key_span.end <= entry.span.end,
                "{}: key span of `{}` is outside its entry",
                path.display(),
                entry.key
            );
            assert!(
                key == entry.key || key.trim_matches(['"', '\'']) == entry.key,
                "{}: key span reads `{key}`, not `{}`",
                path.display(),
                entry.key
            );
            assert!(
                !span.ends_with('\n') && !span.ends_with('\r'),
                "{}: entry `{}` swallowed its newline",
                path.display(),
                entry.key
            );
            // The entry lies inside the block's YAML.
            assert!(
                entry.span.start >= props.yaml.start && entry.span.end <= props.yaml.end,
                "{}: entry `{}` reaches outside the block",
                path.display(),
                entry.key
            );
        }
    }
}

/// A real edit changes exactly the entry it names and nothing else: the bytes
/// before and after that entry are identical.
#[test]
fn an_edit_touches_only_its_own_entry() {
    for path in corpus_notes() {
        let text = std::fs::read_to_string(&path).unwrap();
        let Some(props) = note_properties(&text) else {
            continue;
        };
        let Some(entry) = props.entries.first() else {
            continue;
        };
        let change = Change::Set {
            key: entry.key.clone(),
            value: Value::Text("silica-probe-value".into()),
        };
        let Some(edits) = plan(&text, &props, &[change]) else {
            continue;
        };
        let after = apply(&text, &edits);
        assert_eq!(
            &after[..entry.span.start],
            &text[..entry.span.start],
            "{}: bytes before `{}` moved",
            path.display(),
            entry.key
        );
        let rest_before = &text[entry.span.end..];
        let rest_after = &after[after.len() - rest_before.len()..];
        assert_eq!(
            rest_after,
            rest_before,
            "{}: bytes after `{}` changed",
            path.display(),
            entry.key
        );
        // And the new value reads back.
        let reparsed = note_properties(&after).unwrap();
        assert_eq!(
            reparsed.get(&entry.key).unwrap().value,
            Value::Text("silica-probe-value".into()),
            "{}: `{}` did not read back",
            path.display(),
            entry.key
        );
    }
}

/// Removing every property removes the block and leaves the body untouched,
/// as Obsidian's "Clear file properties" does.
#[test]
fn clearing_a_note_leaves_exactly_its_body() {
    for path in corpus_notes() {
        let text = std::fs::read_to_string(&path).unwrap();
        let Some(props) = note_properties(&text) else {
            continue;
        };
        if props.entries.is_empty() {
            continue;
        }
        let changes: Vec<Change> = props
            .entries
            .iter()
            .map(|p| Change::Remove { key: p.key.clone() })
            .collect();
        let edits = plan(&text, &props, &changes).expect("clearing plans an edit");
        let after = apply(&text, &edits);
        // Everything before the block (a BOM, where there is one) and
        // everything after it survives; only the block goes.
        let expected = format!("{}{}", &text[..props.block.start], &text[props.block.end..]);
        assert_eq!(
            after,
            expected,
            "{}: clearing did not leave the body alone",
            path.display()
        );
        assert!(
            note_properties(&after).is_none() || after.starts_with("---"),
            "{}: a block survived clearing",
            path.display()
        );
    }
}

/// Adding a property leaves every existing byte in place and appends one line.
#[test]
fn adding_a_property_appends_one_line() {
    for path in corpus_notes() {
        let text = std::fs::read_to_string(&path).unwrap();
        let Some(props) = note_properties(&text) else {
            continue;
        };
        if props.get("silica_probe").is_some() {
            continue;
        }
        let change = Change::Set {
            key: "silica_probe".into(),
            value: Value::Number(1.0),
        };
        let edits = plan(&text, &props, &[change]).expect("adding plans an edit");
        let after = apply(&text, &edits);
        assert_eq!(
            after.len(),
            text.len() + props.newline.len() + "silica_probe: 1".len(),
            "{}: adding changed more than one line",
            path.display()
        );
        let reparsed = note_properties(&after).unwrap();
        assert_eq!(
            reparsed.get("silica_probe").unwrap().value,
            Value::Number(1.0)
        );
        // Every key that was there is still there, in the same order.
        let before: Vec<&str> = props.entries.iter().map(|p| p.key.as_str()).collect();
        let now: Vec<&str> = reparsed
            .entries
            .iter()
            .map(|p| p.key.as_str())
            .filter(|k| *k != "silica_probe")
            .collect();
        assert_eq!(before, now, "{}: key order changed", path.display());
    }
}
