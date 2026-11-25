//! The Note composer core plugin's bytes, free of gpui: what a merge writes
//! into the note it merges into, what an extraction leaves behind and where
//! it puts what it took, and the template both wrap their text in.
//!
//! - **The joint** ([`joint`]): the part that comes first is padded to end in
//!   a blank line — nothing when it already ends in `\n\n`, one `\n` after a
//!   single newline, `\n\n` after none — and the part that follows is added
//!   untouched. The padding is a bare `\n` even in a CRLF note.
//! - **Two frontmatters** ([`merge_notes`]) become one, at the top, whichever
//!   way the bodies are joined: the target's keys keep their place, the
//!   source's new keys follow, a scalar clash takes the source's value and a
//!   list clash the union, written as a block list.
//! - **The template** ([`expand_template`]) fills `{{fromTitle}}`,
//!   `{{newTitle}}`, `{{date:FORMAT}}` and `{{content}}`; one without
//!   `{{content}}` gets the content after `\n\n`.
//!
//! Links to a merged note are rewritten by the rename planner,
//! [`MetadataCache::plan_merge_rewrites`], and the link an extraction leaves
//! is written by the caller from [`crate::links::link_path`], the one link
//! writer autocompletion and backlinks already share.

use crate::links::{basename, Rewrite};
use crate::properties::note_properties;
use silica_formats::properties::{apply, plan, Change, Properties, Value};
use silica_templates::{format, Stamp};
use std::ops::Range;

/// `replacementText` in `note-composer.json`: what an extraction leaves in
/// place of the text it took.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Replacement {
    /// "Link to new file", the default.
    #[default]
    Link,
    /// "Embed new file".
    Embed,
    /// "None": the text is simply cut.
    None,
}

impl Replacement {
    /// The config value; absent and unknown values are the default, `link`.
    pub fn from_config(value: Option<&str>) -> Replacement {
        match value {
            Some("embed") => Replacement::Embed,
            Some("none") => Replacement::None,
            _ => Replacement::Link,
        }
    }
}

/// Which way two notes' bodies are joined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// Enter: the merged text after the target's.
    Append,
    /// Shift+Enter: before it ("to merge at top", "to move to top").
    Prepend,
}

/// `first` padded to end in a blank line, then `second` as it is.
///
/// An empty `first` takes no padding: a merge into a note the prompt just
/// created leaves it the source's bytes exactly.
pub fn joint(first: &str, second: &str) -> String {
    if first.is_empty() {
        return second.to_string();
    }
    let pad = if first.ends_with("\n\n") {
        ""
    } else if first.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    let mut out = String::with_capacity(first.len() + pad.len() + second.len());
    out.push_str(first);
    out.push_str(pad);
    out.push_str(second);
    out
}

/// A note's frontmatter block (the fences and the newline after the closing
/// one) and the body after it, with a byte order mark left off the body of
/// a note that has no block.
fn split_note(text: &str) -> (Option<Properties>, &str) {
    match note_properties(text) {
        Some(props) => {
            let end = props.block.end;
            (Some(props), &text[end..])
        }
        None => (None, text.strip_prefix('\u{feff}').unwrap_or(text)),
    }
}

/// The value of a clashing key that one side or the other holds as a list,
/// as list items.
fn items(value: &Value) -> Vec<String> {
    match value {
        Value::List(items) => items.clone(),
        Value::Empty => Vec::new(),
        other => vec![other.display()],
    }
}

/// `target` (a whole note) with `source`'s frontmatter merged into its own:
/// target keys first and in place, a source key it lacks after them, a
/// scalar clash taking the source's value, a list clash becoming the union
/// (the target's items, then the source's not already there) as a block
/// list. Only the entries that change are rewritten.
fn merge_blocks(target: &str, tp: &Properties, sp: &Properties) -> String {
    let mut changes = Vec::new();
    for entry in &sp.entries {
        let value = match tp.get(&entry.key) {
            Some(existing)
                if matches!(existing.value, Value::List(_))
                    || matches!(entry.value, Value::List(_)) =>
            {
                let mut union = items(&existing.value);
                for item in items(&entry.value) {
                    if !union.contains(&item) {
                        union.push(item);
                    }
                }
                Value::List(union)
            }
            _ => entry.value.clone(),
        };
        changes.push(Change::Set {
            key: entry.key.clone(),
            value,
        });
    }
    match plan(target, tp, &changes) {
        Some(edits) => apply(target, &edits),
        None => target.to_string(),
    }
}

/// The note a merge leaves: `target` and `source` (both whole notes) with
/// one frontmatter at the top and their bodies joined by [`joint`] in the
/// order `place` gives. `wrap` is applied to the source's body before the
/// join — the template, when one is set; the identity otherwise.
pub fn merge_notes(
    target: &str,
    source: &str,
    place: Placement,
    wrap: &dyn Fn(&str) -> String,
) -> String {
    let (tp, _) = split_note(target);
    let (sp, source_body) = split_note(source);
    let source_body = wrap(source_body);
    // The target with the source's keys in its block, and where its body
    // starts now.
    let (head, target_body) = match (&tp, &sp) {
        (Some(_), Some(sp)) => {
            let merged = merge_blocks(target, tp.as_ref().unwrap(), sp);
            let end = note_properties(&merged).map(|p| p.block.end).unwrap_or(0);
            (merged[..end].to_string(), merged[end..].to_string())
        }
        (Some(tp), None) => (
            target[..tp.block.end].to_string(),
            target[tp.block.end..].to_string(),
        ),
        // The source's block, as it was written, above a target that had
        // none.
        (None, Some(sp)) => {
            let mut block = source[sp.block.clone()].to_string();
            if !block.ends_with('\n') {
                block.push('\n');
            }
            let body = target.strip_prefix('\u{feff}').unwrap_or(target);
            let bom = if body.len() < target.len() {
                "\u{feff}"
            } else {
                ""
            };
            (format!("{bom}{block}"), body.to_string())
        }
        (None, None) => (String::new(), target.to_string()),
    };
    let body = match place {
        Placement::Append => joint(&target_body, &source_body),
        Placement::Prepend => joint(&source_body, &target_body),
    };
    head + &body
}

/// What a composer template's variables read.
#[derive(Debug, Clone, Copy)]
pub struct TemplateVars<'a> {
    /// `{{fromTitle}}`: the basename of the note the text comes from.
    pub from_title: &'a str,
    /// `{{newTitle}}`: the basename of the note it goes into.
    pub new_title: &'a str,
    /// The instant `{{date:FORMAT}}` formats.
    pub at: Stamp,
}

/// `template` with its four variables filled in around `content`.
///
/// `{{content}}` is the text as it is; a template that never names it gets
/// the content after `\n\n`. The content is inserted, never scanned, so a
/// `{{` in a note stays text. Only the four variables the settings tab
/// names are filled: `{{title}}`, `{{time}}` and a bare `{{date}}` are left
/// as written.
pub fn expand_template(template: &str, content: &str, vars: &TemplateVars<'_>) -> String {
    let mut out = String::with_capacity(template.len() + content.len());
    let mut had_content = false;
    let mut rest = template;
    while let Some(open) = rest.find("{{") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        let Some(close) = after.find("}}") else {
            out.push_str("{{");
            rest = after;
            continue;
        };
        let body = &after[..close];
        let filled = match body {
            "content" => {
                had_content = true;
                Some(content.to_string())
            }
            "fromTitle" => Some(vars.from_title.to_string()),
            "newTitle" => Some(vars.new_title.to_string()),
            _ => body
                .strip_prefix("date:")
                .filter(|f| !f.is_empty())
                .map(|format_text| format(format_text, &vars.at)),
        };
        match filled {
            Some(text) => {
                out.push_str(&text);
                rest = &after[close + 2..];
            }
            None => {
                out.push_str("{{");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    if !had_content {
        out.push_str("\n\n");
        out.push_str(content);
    }
    out
}

/// What one merge writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergePlan {
    /// The target's new bytes.
    pub target: String,
    /// The other notes whose links to the source now point at the target,
    /// with their new bytes. The source's own rewrite is not here: its text
    /// went into the target as written.
    pub others: Vec<Rewrite>,
}

/// The merge of `source` into `target`.
///
/// `rewrites` is the rename planner's answer when the links are to be
/// updated and empty when not; the target's own rewrite, if any, is applied
/// before the source's text joins it, which is why the merged-in self-link
/// `[[src#H]]` stays as written while the target's `[[src]]` becomes
/// `[[dst]]`. `target_text` is empty for a note the prompt creates.
pub fn plan_merge(
    source: &str,
    source_text: &str,
    target: &str,
    target_text: &str,
    place: Placement,
    template: Option<(&str, Stamp)>,
    rewrites: &[Rewrite],
) -> MergePlan {
    let base = rewrites
        .iter()
        .find(|r| r.old_path == target)
        .map(|r| r.text.as_str())
        .unwrap_or(target_text);
    let wrap = |body: &str| match template {
        Some((tpl, at)) => expand_template(
            tpl,
            body,
            &TemplateVars {
                from_title: basename(source),
                new_title: basename(target),
                at,
            },
        ),
        None => body.to_string(),
    };
    MergePlan {
        target: merge_notes(base, source_text, place, &wrap),
        others: rewrites
            .iter()
            .filter(|r| r.old_path != source && r.old_path != target)
            .cloned()
            .collect(),
    }
}

/// What one extraction writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractPlan {
    /// The range of the source to replace, and what goes there.
    pub range: Range<usize>,
    pub replacement: String,
    /// The note the text goes into, and its new bytes.
    pub target: String,
    pub target_text: String,
}

impl ExtractPlan {
    /// The source with the replacement made.
    pub fn applied(&self, source_text: &str) -> String {
        let mut out = source_text.to_string();
        out.replace_range(self.range.clone(), &self.replacement);
        out
    }
}

/// The note settings an extraction reads.
pub struct ExtractOptions<'a> {
    pub replacement: Replacement,
    /// The text the replacement is written as; empty for
    /// [`Replacement::None`], which leaves nothing behind.
    pub link: &'a str,
    /// The template's text, if one is set, and the instant its dates read.
    pub template: Option<(&'a str, Stamp)>,
}

/// Moves `range` of `source_text` (the note at `source`) into `target`:
/// `target_text` is `None` for a note the extraction creates, which then
/// holds the text (through the template) and nothing else, and the note's
/// current bytes otherwise, which the text joins by [`merge_notes`]' rules.
/// `link` must be written against a resolver that already knows `target`,
/// so that it is the shortest one that reaches it.
pub fn plan_extract(
    source: &str,
    source_text: &str,
    range: Range<usize>,
    target: &str,
    target_text: Option<&str>,
    place: Placement,
    opts: &ExtractOptions<'_>,
) -> ExtractPlan {
    let taken = &source_text[range.clone()];
    let content = match opts.template {
        Some((tpl, at)) => expand_template(
            tpl,
            taken,
            &TemplateVars {
                from_title: basename(source),
                new_title: basename(target),
                at,
            },
        ),
        None => taken.to_string(),
    };
    let target_text = merge_notes(target_text.unwrap_or(""), &content, place, &|b| {
        b.to_string()
    });
    let replacement = match opts.replacement {
        Replacement::None => String::new(),
        Replacement::Link | Replacement::Embed => opts.link.to_string(),
    };
    ExtractPlan {
        range,
        replacement,
        target: target.to_string(),
        target_text,
    }
}

/// `(files, links)` the "Update links" notice counts.
pub fn counts(rewrites: &[Rewrite]) -> (usize, usize) {
    (rewrites.len(), rewrites.iter().map(|r| r.links).sum())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(body: &str) -> String {
        body.to_string()
    }

    fn stamp() -> Stamp {
        Stamp {
            year: 2026,
            month: 11,
            day: 9,
            hour: 20,
            minute: 21,
            second: 0,
        }
    }

    #[test]
    fn the_joint_pads_the_first_part_to_a_blank_line() {
        assert_eq!(joint("a", "b"), "a\n\nb");
        assert_eq!(joint("a\n", "b"), "a\n\nb");
        assert_eq!(joint("a\n\n", "b"), "a\n\nb");
        assert_eq!(joint("", "b"), "b");
        // The padding is a bare `\n` even between two CRLF parts.
        assert_eq!(joint("a\r\n", "b\r\n"), "a\r\n\nb\r\n");
    }

    #[test]
    fn both_placements_join_by_the_joint_rule() {
        let src = "src line 1\nsrc line 2\n";
        let dst = "dst line 1\ndst line 2";
        assert_eq!(
            merge_notes(dst, src, Placement::Append, &identity),
            "dst line 1\ndst line 2\n\nsrc line 1\nsrc line 2\n"
        );
        assert_eq!(
            merge_notes(dst, src, Placement::Prepend, &identity),
            "src line 1\nsrc line 2\n\ndst line 1\ndst line 2"
        );
    }

    #[test]
    fn an_empty_target_takes_the_text_unpadded() {
        assert_eq!(
            merge_notes("", "src\n", Placement::Append, &identity),
            "src\n"
        );
        let with_block = "---\nk: v\n---\nbody\n";
        assert_eq!(
            merge_notes("", with_block, Placement::Append, &identity),
            with_block
        );
    }

    #[test]
    fn two_frontmatters_become_one_with_the_targets_keys_first() {
        let src = "---\ntitle: from src\ntags:\n  - s1\n  - shared\naliases: [sa]\nonly: s\n---\nsrc body\n";
        let dst = "---\ntitle: from dst\ntags:\n  - d1\n  - shared\naliases: [da]\n---\ndst body\n";
        assert_eq!(
            merge_notes(dst, src, Placement::Append, &identity),
            "---\ntitle: from src\ntags:\n  - d1\n  - shared\n  - s1\naliases:\n  - da\n  - sa\nonly: s\n---\ndst body\n\nsrc body\n"
        );
        // The source's block lands above a target that had none.
        let src_only = "---\nsrckey: s\n---\nsrc body\n";
        assert_eq!(
            merge_notes("dst body\n", src_only, Placement::Append, &identity),
            "---\nsrckey: s\n---\ndst body\n\nsrc body\n"
        );
    }

    #[test]
    fn a_template_fills_its_four_variables_and_keeps_the_rest() {
        let vars = TemplateVars {
            from_title: "src",
            new_title: "dst",
            at: stamp(),
        };
        assert_eq!(
            expand_template(
                "T:{{fromTitle}}|{{newTitle}}|{{date:YYYY}}|{{content}}|END\n",
                "body",
                &vars
            ),
            "T:src|dst|2026|body|END\n"
        );
        // No `{{content}}`: the text follows after two newlines.
        assert_eq!(
            expand_template("T:{{fromTitle}}|END\n", "PICK ME", &vars),
            "T:src|END\n\n\nPICK ME"
        );
        // The content is inserted, never scanned.
        assert_eq!(
            expand_template("{{content}} {{title}} {{date}}", "{{fromTitle}}", &vars),
            "{{fromTitle}} {{title}} {{date}}"
        );
    }

    #[test]
    fn a_merge_wraps_the_source_in_the_template() {
        let tpl = "T:{{fromTitle}}|{{newTitle}}|{{date:YYYY}}|{{content}}|END\n";
        let plan = plan_merge(
            "tpl-src.md",
            "tpl src\n",
            "tpl-dst.md",
            "tpl dst\n",
            Placement::Append,
            Some((tpl, stamp())),
            &[],
        );
        assert_eq!(
            plan.target,
            "tpl dst\n\nT:tpl-src|tpl-dst|2026|tpl src\n|END\n"
        );
    }

    fn opts(replacement: Replacement, link: &'static str) -> ExtractOptions<'static> {
        ExtractOptions {
            replacement,
            link,
            template: None,
        }
    }

    #[test]
    fn an_extraction_leaves_the_link_it_is_given_or_nothing() {
        let source = "before\nPICK ME\nafter\n";
        let cases = [
            (Replacement::Link, "[[new]]", "before\n[[new]]\nafter\n"),
            (Replacement::Embed, "![[new]]", "before\n![[new]]\nafter\n"),
            (Replacement::None, "[[new]]", "before\n\nafter\n"),
        ];
        for (replacement, link, after) in cases {
            let plan = plan_extract(
                "s.md",
                source,
                7..14,
                "new.md",
                None,
                Placement::Append,
                &opts(replacement, link),
            );
            assert_eq!(plan.applied(source), after);
            assert_eq!(plan.target_text, "PICK ME");
        }
    }

    #[test]
    fn an_extraction_into_an_existing_note_joins_it_by_the_merge_rule() {
        let source = "before\nPICK ME\nafter\n";
        let plan = plan_extract(
            "s.md",
            source,
            7..14,
            "t.md",
            Some("existing target\n"),
            Placement::Append,
            &opts(Replacement::Link, "[[t]]"),
        );
        assert_eq!(plan.target_text, "existing target\n\nPICK ME");
        assert_eq!(plan.applied(source), "before\n[[t]]\nafter\n");
    }

    #[test]
    fn an_extracted_selection_goes_through_the_template() {
        let tpl = "T:{{fromTitle}}|{{newTitle}}|{{date:YYYY}}|{{content}}|END\n";
        let mut o = opts(Replacement::Link, "[[tpl-new]]");
        o.template = Some((tpl, stamp()));
        let plan = plan_extract(
            "tpl-split.md",
            "before\nPICK ME\nafter\n",
            7..14,
            "tpl-new.md",
            None,
            Placement::Append,
            &o,
        );
        assert_eq!(
            plan.target_text,
            "T:tpl-split|tpl-new|2026|PICK ME|END\n"
        );
    }

    #[test]
    fn replacement_text_defaults_to_a_link() {
        assert_eq!(Replacement::from_config(None), Replacement::Link);
        assert_eq!(Replacement::from_config(Some("embed")), Replacement::Embed);
        assert_eq!(Replacement::from_config(Some("none")), Replacement::None);
        assert_eq!(Replacement::from_config(Some("??")), Replacement::Link);
    }
}
