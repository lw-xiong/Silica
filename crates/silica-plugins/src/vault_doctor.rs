//! Vault doctor: a health report no Obsidian plugin ships by default.
//!
//! Silica already computes everything a vault audit needs — the metadata
//! cache resolves links, indexes tags and frontmatter — but nothing surfaces
//! the failures a long-lived vault accumulates. `silica:vault-doctor` reads
//! the notes through the plugin context, indexes them with the same
//! `silica-cache` engine the app uses, and writes one report note:
//!
//! - **Broken links** — every unresolved wikilink or markdown link, with the
//!   notes that carry it. These are the links clicking does nothing for.
//! - **Ambiguous basenames** — files sharing one basename across folders. A
//!   bare `[[Note]]` resolves to one of them by Obsidian's tie-break rules;
//!   a reader who wrote the link may have meant the other.
//! - **Orphan notes** — notes no other note links to. The note on screen is
//!   exempt: it is not an orphan merely for being open.
//! - **Empty notes** — files whose body is whitespace after frontmatter.
//!
//! The report is written to `Vault doctor.md` at the vault root as plain
//! Markdown — wikilinks, not paths — so the report is itself navigable and
//! disappears with a delete like any note. Nothing else is written.

use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};

use silica_cache::{index_markdown, CachedMetadata, LinkResolver};

use crate::native::{NativeCommand, Plugin, PluginContext};

pub struct VaultDoctor;

impl Plugin for VaultDoctor {
    fn id(&self) -> &'static str {
        "silica:vault-doctor"
    }

    fn name(&self) -> &'static str {
        "Vault doctor"
    }

    fn commands(&self) -> Vec<NativeCommand> {
        vec![NativeCommand {
            id: "vault-doctor:run".into(),
            name: "Vault doctor: health report".into(),
        }]
    }

    fn run_command(
        &self,
        id: &str,
        _value: Option<&Value>,
        ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        if id != "vault-doctor:run" {
            return Err(crate::native::unknown_command(id));
        }
        let report = build_report(ctx)?;
        const REPORT: &str = "Vault doctor.md";
        (ctx.write_file)(REPORT, &report)?;
        (ctx.open_note)(REPORT)?;
        Ok(())
    }
}

/// The audit behind one report.
struct Audit {
    /// Unresolved link text as written -> the notes carrying it.
    broken: BTreeMap<String, Vec<String>>,
    /// Lowercased basename -> the files sharing it; groups of one file out.
    ambiguous: Vec<(String, Vec<String>)>,
    /// Notes nothing links to.
    orphans: Vec<String>,
    /// Notes with an empty body.
    empty: Vec<String>,
    /// How many notes were scanned.
    scanned: usize,
}

/// Reads every markdown note through the context and audits it.
fn audit(ctx: &PluginContext<'_>) -> Result<Audit, String> {
    let mut notes: Vec<String> = Vec::new();
    collect_markdown(ctx, ".", &mut notes)?;
    notes.sort();
    notes.dedup();

    let mut texts: HashMap<String, String> = HashMap::new();
    let mut metas: Vec<(String, CachedMetadata)> = Vec::new();
    for path in &notes {
        let text = (ctx.read_file)(path)?;
        metas.push((path.clone(), index_markdown(&text)));
        texts.insert(path.clone(), text);
    }
    let resolver = LinkResolver::new(notes.iter().map(String::as_str));

    let current = (ctx.current_note)().unwrap_or_default();
    let mut broken: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut linked: HashSet<String> = HashSet::new();
    let mut basenames: HashMap<String, Vec<String>> = HashMap::new();
    let mut empty: Vec<String> = Vec::new();

    for (path, meta) in &metas {
        if let Some(base) = base_of(path) {
            basenames.entry(base).or_default().push(path.clone());
        }
        // * `body_is_empty` already strips the frontmatter, so a note that is
        //   nothing but properties counts as empty here too.
        if body_is_empty(texts.get(path).map(String::as_str)) {
            empty.push(path.clone());
        }
        for link in &meta.links {
            match resolver.resolve(&link.link, path) {
                Some(target) => {
                    linked.insert(target.to_string());
                }
                None => {
                    broken
                        .entry(link.original.clone())
                        .or_default()
                        .push(path.clone());
                }
            }
        }
        for link in &meta.embeds {
            match resolver.resolve(&link.link, path) {
                Some(target) => {
                    linked.insert(target.to_string());
                }
                // * Only note embeds can be broken here: this audit indexes
                //   markdown, so `![[picture.png]]` is an attachment it never
                //   looked for, not a dangling link.
                None if is_note_target(&link.link) => {
                    broken
                        .entry(link.original.clone())
                        .or_default()
                        .push(path.clone());
                }
                None => {}
            }
        }
    }

    let ambiguous: Vec<(String, Vec<String>)> = {
        let mut keys: Vec<&String> = basenames
            .iter()
            .filter(|(_, v)| v.len() > 1)
            .map(|(k, _)| k)
            .collect();
        keys.sort();
        keys.into_iter()
            .map(|k| {
                let v = basenames.get(k).cloned().unwrap_or_default();
                (k.clone(), v)
            })
            .collect()
    };

    let orphans: Vec<String> = notes
        .iter()
        .filter(|p| !linked.contains(p.as_str()))
        .filter(|p| p.as_str() != current)
        .cloned()
        .collect();

    Ok(Audit {
        broken,
        ambiguous,
        orphans,
        empty,
        scanned: notes.len(),
    })
}

/// The full report as one Markdown note.
fn build_report(ctx: &PluginContext<'_>) -> Result<String, String> {
    let audit = audit(ctx)?;
    let mut out = String::from(
        "# Vault doctor\n\nGenerated by the `Vault doctor: health report` command. \
Delete this note freely — the next run recreates it.\n\n",
    );
    out.push_str(&format!(
        "**{} note{} scanned** · {} broken link target{}, {} ambiguous basename group{}, \
{} orphan note{}, {} empty note{}\n",
        audit.scanned,
        plural(audit.scanned),
        audit.broken.len(),
        plural(audit.broken.len()),
        audit.ambiguous.len(),
        plural(audit.ambiguous.len()),
        audit.orphans.len(),
        plural(audit.orphans.len()),
        audit.empty.len(),
        plural(audit.empty.len()),
    ));
    out.push_str("\n---\n\n");

    if audit.broken.is_empty() {
        out.push_str("## Broken links\n\nNone.\n\n");
    } else {
        out.push_str(
            "## Broken links\n\nEvery link below resolves to no file in the vault. \
Fix the name, create the note, or remove the link.\n\n",
        );
        for (target, sources) in &audit.broken {
            let from: Vec<String> = sources
                .iter()
                .map(|s| format!("[[{}]]", strip_md(s)))
                .collect();
            // * `original` is the link text as written; some parsers keep the
            //   brackets, so trim before wrapping them back on.
            let target = target
                .trim()
                .trim_start_matches("![[")
                .trim_start_matches("[[");
            let target = target.trim_end_matches("]]");
            out.push_str(&format!("- `[[{target}]]` — from {}\n", from.join(", ")));
        }
        out.push('\n');
    }

    if audit.ambiguous.is_empty() {
        out.push_str("## Ambiguous basenames\n\nNone.\n\n");
    } else {
        out.push_str(
            "## Ambiguous basenames\n\nThese files share one basename, so a bare \
`[[Name]]` resolves to whichever one Obsidian's tie-break rules prefer — not necessarily \
the one the writer meant.\n\n",
        );
        for (_, paths) in &audit.ambiguous {
            let list: Vec<String> = paths
                .iter()
                .map(|p| format!("[[{}]]", strip_md(p)))
                .collect();
            out.push_str(&format!("- {}\n", list.join(" · ")));
        }
        out.push('\n');
    }

    if audit.orphans.is_empty() {
        out.push_str("## Orphan notes\n\nNone.\n\n");
    } else {
        out.push_str(
            "## Orphan notes\n\nNo note links to these. They may be deliberately \
standalone.\n\n",
        );
        for p in &audit.orphans {
            out.push_str(&format!("- [[{}]]\n", strip_md(p)));
        }
        out.push('\n');
    }

    if audit.empty.is_empty() {
        out.push_str("## Empty notes\n\nNone.\n");
    } else {
        out.push_str("## Empty notes\n\nNothing after the properties block.\n\n");
        for p in &audit.empty {
            out.push_str(&format!("- [[{}]]\n", strip_md(p)));
        }
    }
    Ok(out)
}

/// Depth-first markdown walk over `list_dir`; folders end with `/`.
fn collect_markdown(
    ctx: &PluginContext<'_>,
    dir: &str,
    out: &mut Vec<String>,
) -> Result<(), String> {
    let entries = (ctx.list_dir)(dir).map_err(|e| format!("cannot read {dir}: {e}"))?;
    for entry in entries {
        // * `.obsidian`, `.trash`, `.git`: configuration and trash, not notes.
        if entry.starts_with('.') {
            continue;
        }
        if let Some(folder) = entry.strip_suffix('/') {
            let child = if dir == "." {
                folder.to_string()
            } else {
                format!("{dir}/{folder}")
            };
            collect_markdown(ctx, &child, out)?;
        } else if entry.ends_with(".md") {
            let full = if dir == "." {
                entry.clone()
            } else {
                format!("{dir}/{entry}")
            };
            out.push(full);
        }
    }
    Ok(())
}

/// Whitespace-only body after the frontmatter, if any.
fn body_is_empty(text: Option<&str>) -> bool {
    let Some(text) = text else {
        return true;
    };
    let body = match text.strip_prefix("---\n") {
        Some(rest) => match rest.find("\n---") {
            Some(i) => &rest[i + 4..],
            // * An unterminated fence is body, not frontmatter.
            None => text,
        },
        None => text,
    };
    body.trim().is_empty()
}

fn base_of(path: &str) -> Option<String> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let stem = name.strip_suffix(".md")?;
    Some(stem.to_lowercase())
}

/// A link that names a note rather than an attachment.
fn is_note_target(link: &str) -> bool {
    let name = link.split('#').next().unwrap_or(link);
    let leaf = name.rsplit('/').next().unwrap_or(name);
    match leaf.rsplit_once('.') {
        Some((_, ext)) => ext.eq_ignore_ascii_case("md"),
        None => !leaf.is_empty(),
    }
}

fn strip_md(path: &str) -> &str {
    path.strip_suffix(".md").unwrap_or(path)
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_of_lowercases_and_strips() {
        assert_eq!(base_of("Sub/Note.md").as_deref(), Some("note"));
        assert_eq!(base_of("picture.png"), None);
    }

    #[test]
    fn is_note_target_separates_notes_from_attachments() {
        assert!(is_note_target("Note"));
        assert!(is_note_target("Sub/Note.md"));
        assert!(is_note_target("Note#Heading"));
        assert!(!is_note_target("picture.png"));
        assert!(!is_note_target(""));
    }

    #[test]
    fn strip_md_keeps_other_extensions() {
        assert_eq!(strip_md("Note.md"), "Note");
        assert_eq!(strip_md("pic.png"), "pic.png");
    }

    #[test]
    fn body_is_empty_sees_through_frontmatter() {
        assert!(body_is_empty(Some("")));
        assert!(body_is_empty(Some("   \n\t")));
        assert!(body_is_empty(Some("---\ntitle: x\n---\n  \n")));
        assert!(!body_is_empty(Some("---\ntitle: x\n---\n# Hello\n")));
        assert!(!body_is_empty(Some("# Hello\n")));
        // * An unterminated fence is body, not frontmatter.
        assert!(!body_is_empty(Some("---\ntitle: x\n")));
    }
}
