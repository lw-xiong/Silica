//! The base model against what Obsidian 1.14.5 wrote, observed over CDP on
//! 2026-10-25 (`tools/obsidian-probe/bases.mjs`, raw run in
//! `tools/obsidian-probe/results/2026-10-25-bases.json`). `scrambled.in.base`
//! under `tests/base` is the file the probe wrote into a scratch vault, and
//! `scrambled.out.base` beside it is what Obsidian wrote back when the view
//! was closed — 1352 bytes in, 1256 out, with nothing edited.
//!
//! There is no writer yet, so what these assert is the half the model owes it:
//! that the key order the model computes is the key order Obsidian wrote, read
//! back out of the observed bytes rather than copied from the prose. The
//! writer and its byte-for-byte gate are the next commit.

use std::path::PathBuf;

use silica_formats::base::{Base, View, DEFAULT_VIEW_NAME, DEFAULT_VIEW_TYPE};

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/base")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The keys of a block in `text` at exactly `indent` spaces, in the order they
/// appear — the written order, taken from the bytes themselves so that these
/// tests cannot drift from the run by being retyped.
fn keys_at(text: &str, indent: usize) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let depth = line.len() - line.trim_start().len();
        if depth != indent {
            continue;
        }
        let trimmed = line.trim_start();
        // A key line, not a sequence item and not a comment.
        if trimmed.starts_with('-') || trimmed.starts_with('#') {
            continue;
        }
        if let Some((key, _)) = trimmed.split_once(':') {
            if !key.is_empty() && !key.contains(' ') {
                out.push(key.to_string());
            }
        }
    }
    out
}

/// The top level: the preserved keys first in their file order, then the
/// serializer's own. `summaries` is in both and keeps its file position;
/// `filters` is consumed and does not.
#[test]
fn the_top_level_key_order_is_the_one_obsidian_wrote() {
    let base = Base::from_yaml(&fixture("scrambled.in.base")).unwrap();
    let written = keys_at(&fixture("scrambled.out.base"), 0);
    assert_eq!(
        written,
        [
            "summaries",
            "unknownTopKey",
            "filters",
            "formulas",
            "properties",
            "views",
            "newItemFolder",
            "newItemTemplate"
        ],
        "the fixture is not the file this test was written against"
    );
    assert_eq!(base.key_order(), written);
}

/// A view's own fields in the serializer's order, then its unknown keys — and
/// every layout key is an unknown key, so `rowHeight` and a plugin's key sort
/// together at the end rather than among the documented ones.
#[test]
fn a_views_key_order_is_the_one_obsidian_wrote() {
    let base = Base::from_yaml(&fixture("scrambled.in.base")).unwrap();
    let out = fixture("scrambled.out.base");
    // The first view's block: from its `- type:` line to the next one.
    let first = out
        .split("  - type:")
        .nth(1)
        .expect("a first view in the written bytes");
    let mut written = vec!["type".to_string()];
    written.extend(keys_at(first, 4));
    assert_eq!(
        written,
        [
            "type",
            "name",
            "groupBy",
            "order",
            "sort",
            "limit",
            "summaries",
            "unknownViewKey",
            "nestedUnknown"
        ]
    );
    assert_eq!(base.views[0].key_order(), written);
}

/// A property writes its unknown keys before `displayName`, which is the
/// reverse of the order the fixture had them in.
#[test]
fn a_propertys_key_order_is_the_one_obsidian_wrote() {
    let base = Base::from_yaml(&fixture("scrambled.in.base")).unwrap();
    let input = fixture("scrambled.in.base");
    assert!(
        input.contains("    displayName: Status\n    unknownPropKey: kept"),
        "the fixture no longer has displayName first, so this proves nothing"
    );
    assert_eq!(
        base.properties["status"].key_order(),
        ["unknownPropKey", "displayName"]
    );
}

/// `note.unused: {}` is in the file and not in the written bytes: an empty
/// config serializes to null and a null is left out of the map.
#[test]
fn an_empty_property_config_is_dropped() {
    let base = Base::from_yaml(&fixture("scrambled.in.base")).unwrap();
    let out = fixture("scrambled.out.base");
    assert!(fixture("scrambled.in.base").contains("note.unused"));
    assert!(!out.contains("note.unused"));
    assert!(base.properties["note.unused"].is_dropped());
}

/// Statement text survives byte for byte. Only the YAML quoting around it
/// changes, which is the emitter's business and not the model's.
#[test]
fn statement_text_survives_byte_for_byte() {
    let base = Base::from_yaml(&fixture("scrambled.in.base")).unwrap();
    // A double space inside a string literal, and a trailing space inside one.
    assert_eq!(base.formulas["spaced"], r#"type + " "  + file.name"#);
    assert_eq!(base.formulas["trailingSpace"], r#"status + "x" "#);
    let out = fixture("scrambled.out.base");
    for statement in base.formulas.values() {
        assert!(
            out.contains(statement.as_str()),
            "{statement:?} is not in the bytes Obsidian wrote"
        );
    }
}

/// Every key of the written file round-trips into the model: nothing Obsidian
/// kept is dropped on the way in.
#[test]
fn what_obsidian_wrote_parses_back_to_the_same_model() {
    let from_in = Base::from_yaml(&fixture("scrambled.in.base")).unwrap();
    let from_out = Base::from_yaml(&fixture("scrambled.out.base")).unwrap();
    // The one difference the canonicalization makes to the *model* is the
    // dropped empty property config; everything else is byte noise the model
    // does not hold.
    assert!(from_in.properties.contains_key("note.unused"));
    assert!(!from_out.properties.contains_key("note.unused"));
    assert_eq!(from_in.key_order(), from_out.key_order());
    assert_eq!(from_in.formulas, from_out.formulas);
    assert_eq!(from_in.summaries, from_out.summaries);
    assert_eq!(from_in.filters, from_out.filters);
    assert_eq!(from_in.views, from_out.views);
    assert_eq!(from_in.new_item_folder, from_out.new_item_folder);
    assert_eq!(from_in.new_item_template, from_out.new_item_template);
}

// ---------------------------------------------------------------------------
// The writer, byte for byte
// ---------------------------------------------------------------------------
//
// Each pair below is one file the probe wrote into a scratch vault and the
// bytes Obsidian left in its place. Parsing the first and writing it has to
// produce the second exactly, which is the whole gate: a base Silica saves is
// a base Obsidian would have saved.

/// Every documented key, scrambled, with comments at three depths, flow
/// collections, four-space indentation and both quoting styles. Nothing was
/// edited: this is what a close alone does.
#[test]
fn an_unedited_close_writes_the_canonical_form() {
    let base = Base::from_yaml(&fixture("scrambled.in.base")).unwrap();
    assert_eq!(base.to_yaml(), fixture("scrambled.out.base"));
}

/// CRLF throughout and a UTF-8 BOM are both dropped, and both files come back
/// as the same canonical bytes as the LF original — which is the strongest
/// form this test can take: three different inputs, one output.
#[test]
fn line_endings_and_a_bom_are_canonicalized_away() {
    let crlf = Base::from_yaml(&fixture("crlf.in.base")).unwrap();
    assert_eq!(crlf.to_yaml(), fixture("crlf.out.base"));

    let bom_text = fixture("bom.in.base");
    let bom = Base::from_yaml(bom_text.trim_start_matches('\u{feff}')).unwrap();
    assert_eq!(bom.to_yaml(), fixture("bom.out.base"));

    assert_eq!(fixture("crlf.out.base"), fixture("scrambled.out.base"));
    assert_eq!(fixture("bom.out.base"), fixture("scrambled.out.base"));
}

/// A file with no trailing newline gains one, and gains nothing else.
#[test]
fn a_missing_trailing_newline_is_added() {
    let base = Base::from_yaml(&fixture("no-newline.in.base")).unwrap();
    assert_eq!(base.to_yaml(), fixture("no-newline.out.base"));
    assert!(!fixture("no-newline.in.base").ends_with('\n'));
}

/// A file already in the canonical form is written back identically, which is
/// why Obsidian skips the write for it: the mtime did not move on close.
#[test]
fn an_already_canonical_file_is_unchanged() {
    for name in ["minimal", "layouts", "edited", "default-view"] {
        let file = format!("{name}.out.base");
        let base = Base::from_yaml(&fixture(&file)).unwrap();
        assert_eq!(base.to_yaml(), fixture(&file), "{file} is not idempotent");
    }
    assert_eq!(fixture("minimal.in.base"), fixture("minimal.out.base"));
}

/// A statement the expression parser rejects is not a parse error: the file
/// is canonicalized like any other and the statement's text is kept intact.
#[test]
fn an_unparseable_statement_is_kept_verbatim() {
    for name in ["broken-filter", "broken-formula", "broken-summary"] {
        let base = Base::from_yaml(&fixture(&format!("{name}.in.base"))).unwrap();
        assert_eq!(
            base.to_yaml(),
            fixture(&format!("{name}.out.base")),
            "{name} did not canonicalize the way Obsidian did"
        );
    }
}

/// A view's type changed through the UI: the one edit, and nothing else moves.
#[test]
fn a_layout_change_writes_only_its_own_key() {
    let mut base = Base::from_yaml(&fixture("layout-change.in.base")).unwrap();
    base.views[0].kind = "kanban".to_string();
    assert_eq!(base.to_yaml(), fixture("layout-change.out.base"));
}

/// The view Obsidian gives a base that has none. The emitter does not add it —
/// that is the view layer's job — but the bytes it writes for one must match.
#[test]
fn the_default_view_writes_obsidians_bytes() {
    let base = Base {
        views: vec![View {
            kind: DEFAULT_VIEW_TYPE.to_string(),
            name: DEFAULT_VIEW_NAME.to_string(),
            ..View::default()
        }],
        ..Base::default()
    };
    assert_eq!(base.to_yaml(), fixture("default-view.out.base"));
}

/// The four files Obsidian refused, and left untouched. Each must be an error
/// here too: a writer that accepted one would write bytes Obsidian never would.
#[test]
fn the_files_obsidian_refused_are_refused_here() {
    let cases = [
        ("no-name.in.base", "Missing or invalid \"name\" in view 1"),
        (
            "empty-name.in.base",
            "Missing or invalid \"name\" in view 1",
        ),
        (
            "filter-extra.in.base",
            "\"filters\" may only have one of an \"and\", \"or\", or \"not\" keys.",
        ),
        // Two faults at once — a stray key beside `and` and an empty view
        // name. Obsidian reports the name, so the views are checked first.
        ("oddities.in.base", "Missing or invalid \"name\" in view 1"),
    ];
    for (file, message) in cases {
        let err = Base::from_yaml(&fixture(file))
            .err()
            .unwrap_or_else(|| panic!("{file} parsed, but Obsidian refused it"));
        assert_eq!(err.to_string(), message, "{file}");
    }
}
