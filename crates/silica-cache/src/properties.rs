//! The properties block of one note.
//!
//! `silica-formats` owns the property model and the edit planner but does not
//! depend on the parser; this is the one place that hands it the frontmatter
//! span `silica-markdown` found, so the editor, the reading view and the
//! commands all read a note's properties the same way.

use silica_formats::properties::{parse, Properties};
use silica_markdown::extract_frontmatter;

/// The properties of `text`, or `None` when the note has no block.
///
/// A UTF-8 BOM before the opening fence is tolerated, as `extract_frontmatter`
/// does. Obsidian 1.14.4 does not: it reads such a note as having no
/// properties and prepends a *second* block when one is added, losing the
/// first. Silica recognises the block and edits it in place, keeping the BOM.
pub fn note_properties(text: &str) -> Option<Properties> {
    let fm = extract_frontmatter(text)?;
    // `fm.start` is the opening fence; the YAML runs from just after its line
    // to the start of the closing fence line.
    let yaml_start = text[fm.start..].find('\n')? + fm.start + 1;
    let yaml_end = yaml_start + fm.yaml.len();
    Some(parse(text, yaml_start..yaml_end, fm.start..fm.end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use silica_formats::properties::{apply, plan, Change, Value};

    #[test]
    fn reads_a_block_with_its_spans() {
        let text = "---\ntitle: Hi\ntags: [a]\n---\n# Body\n";
        let p = note_properties(text).unwrap();
        assert_eq!(&text[p.yaml.clone()], "title: Hi\ntags: [a]\n");
        assert_eq!(&text[p.block.clone()], "---\ntitle: Hi\ntags: [a]\n---\n");
        assert_eq!(p.entries.len(), 2);
        assert_eq!(&text[p.entries[1].span.clone()], "tags: [a]");
    }

    #[test]
    fn no_block_is_none() {
        assert!(note_properties("# Body\n").is_none());
        assert!(note_properties("").is_none());
        // Unterminated, and `...` does not close it.
        assert!(note_properties("---\na: 1\n").is_none());
        assert!(note_properties("---\na: 1\n...\nbody\n").is_none());
    }

    #[test]
    fn an_empty_block_has_no_entries_but_a_yaml_range() {
        let text = "---\n---\nbody\n";
        let p = note_properties(text).unwrap();
        assert!(p.entries.is_empty());
        assert_eq!(&text[p.yaml.clone()], "");
        assert_eq!(p.yaml.start, 4);
    }

    #[test]
    fn a_bom_keeps_its_bytes_through_an_edit() {
        let text = "\u{feff}---\ntitle: Hi\n---\nbody\n";
        let p = note_properties(text).unwrap();
        assert_eq!(&text[p.entries[0].span.clone()], "title: Hi");
        let edits = plan(
            text,
            &p,
            &[Change::Set {
                key: "added".into(),
                value: Value::Text("v".into()),
            }],
        )
        .unwrap();
        assert_eq!(
            apply(text, &edits),
            "\u{feff}---\ntitle: Hi\nadded: v\n---\nbody\n"
        );
    }

    #[test]
    fn a_crlf_note_keeps_crlf() {
        let text = "---\r\ntitle: Hi\r\n---\r\nbody\r\n";
        let p = note_properties(text).unwrap();
        assert_eq!(p.newline, "\r\n");
        let edits = plan(
            text,
            &p,
            &[Change::Set {
                key: "n".into(),
                value: Value::Number(1.0),
            }],
        )
        .unwrap();
        assert_eq!(
            apply(text, &edits),
            "---\r\ntitle: Hi\r\nn: 1\r\n---\r\nbody\r\n"
        );
    }
}
