//! Link text generation and link rewriting after a rename or move.
//!
//! Every rule here was observed in Obsidian 1.14.4 by renaming files through
//! `fileManager.renameFile` in a probe vault and diffing the notes (see
//! `docs/silica/Development.md`, "File operations"):
//!
//! - Every link that resolved to a renamed file is regenerated with the
//!   vault's `newLinkFormat` (`shortest`, `relative`, `absolute`), whatever
//!   form it was written in: `[[a]]`, `[[A.md]]`, `[[folder/A]]`, `[A](./A.md)`
//!   and `[A](/A.md)` all come out the same way. A link whose written path
//!   already equals the generated one is left alone byte for byte.
//! - The link style is kept: wikilinks stay wikilinks (`![[` embeds too),
//!   markdown links stay markdown links, an angle-bracketed destination
//!   stays angle-bracketed, otherwise spaces become `%20` and nothing else is
//!   encoded. Markdown destinations keep the `.md` extension, wikilinks drop
//!   it; other file kinds keep their extension in both.
//! - The heading or block subpath is kept (trailing whitespace trimmed); the
//!   alias after `|` is kept verbatim, except that an alias equal to the old
//!   file's basename, and different from the written path, becomes the new
//!   basename (`[[Sub/S|S]]` -> `[[S2|S2]]`, but `[[S|S]]` -> `[[S2|S]]`).
//!   A markdown link's text becomes the new basename when it was the old
//!   basename, or the new link path when it was the old destination without
//!   its extension (`[../A](../A.md)` -> `[A2](A2.md)` in shortest format).
//! - Links inside code spans and fenced code are not links and never change.
//!   Reference-style links (`[text][label]`, `[label]: A.md`) are not updated.
//! - Links inside a moved note itself are regenerated only when they would
//!   stop resolving to the same file (`[A](./A.md)` or `[B](../B.md)` after
//!   the note moved into another folder); the others are left alone.
//! - Frontmatter wikilinks (`related: "[[A]]"`) are updated too. Obsidian
//!   re-serializes the whole properties block when it does; Silica replaces
//!   the link text in place so the rest of the YAML keeps its bytes.

use crate::resolver::folder_of;
use crate::{CachedMetadata, LinkResolver, MetadataCache};
use silica_formats::FileKind;
use std::collections::{BTreeSet, HashMap};

/// `newLinkFormat` in `app.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LinkFormat {
    /// The basename when it is unique in the vault, else the full path.
    #[default]
    Shortest,
    /// Relative to the note the link is written in (`../Other/Note`).
    Relative,
    /// The full vault path.
    Absolute,
}

impl LinkFormat {
    /// Parses the config value; `None` and unknown strings are Obsidian's
    /// default, `shortest`.
    pub fn from_config(value: Option<&str>) -> LinkFormat {
        match value {
            Some("relative") => LinkFormat::Relative,
            Some("absolute") => LinkFormat::Absolute,
            _ => LinkFormat::Shortest,
        }
    }
}

/// The file name without folder or extension (`Sub/Note.md` -> `Note`,
/// `pic.png` -> `pic`), Obsidian's `TFile.basename`.
pub fn basename(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => name,
    }
}

/// A markdown file's path without `.md`; other files keep their extension.
fn link_display_path(path: &str) -> &str {
    if FileKind::from_path(path).is_markdown() {
        path.strip_suffix(".md")
            .or_else(|| path.strip_suffix(".MD"))
            .unwrap_or(path)
    } else {
        path
    }
}

/// `path` relative to `folder`, climbing with `..` as needed.
fn relative_path(folder: &str, path: &str) -> String {
    let from: Vec<&str> = folder.split('/').filter(|p| !p.is_empty()).collect();
    let to: Vec<&str> = path.split('/').collect();
    let common = from
        .iter()
        .zip(to.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let mut out: Vec<&str> = vec![".."; from.len() - common];
    out.extend_from_slice(&to[common..]);
    out.join("/")
}

/// The path part of a link to `target` written in `source`, as Obsidian's
/// `fileToLinktext` generates it: without `.md` for notes, with the
/// extension for everything else, in the given format.
pub fn link_path(
    resolver: &LinkResolver,
    target: &str,
    source: &str,
    format: LinkFormat,
) -> String {
    let display = link_display_path(target);
    match format {
        LinkFormat::Absolute => display.to_string(),
        LinkFormat::Relative => relative_path(folder_of(source), display),
        LinkFormat::Shortest => {
            let short = display.rsplit('/').next().unwrap_or(display);
            if resolver.resolve(short, source) == Some(target) {
                short.to_string()
            } else {
                display.to_string()
            }
        }
    }
}

/// Obsidian encodes only spaces in a markdown link destination.
fn encode_destination(path: &str) -> String {
    path.replace(' ', "%20")
}

/// One note whose text changes because of a rename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rewrite {
    /// The note's path before the rename batch.
    pub old_path: String,
    /// The note's path after it (the same unless the note itself moved).
    pub new_path: String,
    pub text: String,
    /// How many links changed.
    pub links: usize,
}

struct Edit {
    start: usize,
    end: usize,
    text: String,
}

/// What a link occurrence turns into after the rename, if it changes.
struct Target<'a> {
    old_target: &'a str,
    new_target: &'a str,
    new_path: String,
}

/// Rewrites the links of one note after `renames` (old path -> new path)
/// were applied to the vault. `source_old` is the note's path when `text`
/// was indexed into `meta`, `source_new` its path now; `before` resolves
/// paths in the old vault and `after` in the new one. Returns the new text
/// and the number of links changed, or `None` when nothing changes.
#[allow(clippy::too_many_arguments)]
pub fn rewrite_note(
    text: &str,
    meta: &CachedMetadata,
    source_old: &str,
    source_new: &str,
    renames: &HashMap<String, String>,
    before: &LinkResolver,
    after: &LinkResolver,
    format: LinkFormat,
) -> Option<(String, usize)> {
    let target_of = |link: &str| -> Option<Target<'_>> {
        let old_target = before.resolve(link, source_old)?;
        let renamed = renames.get(old_target);
        let new_target = renamed.map(String::as_str).unwrap_or(old_target);
        if renamed.is_none() {
            // Only a note that moved can break its own links.
            if source_old == source_new || after.resolve(link, source_new) == Some(new_target) {
                return None;
            }
        }
        Some(Target {
            old_target,
            new_target,
            new_path: link_path(after, new_target, source_new, format),
        })
    };

    let mut edits: Vec<Edit> = Vec::new();
    for link in meta.links.iter().chain(meta.embeds.iter()) {
        let Some(target) = target_of(&link.link) else {
            continue;
        };
        let original = &link.original;
        let rebuilt = if original.starts_with("[[") || original.starts_with("![[") {
            rebuild_wikilink(original, &target)
        } else {
            rebuild_markdown_link(original, &target)
        };
        if let Some(new) = rebuilt {
            edits.push(Edit {
                start: link.position.start.offset,
                end: link.position.end.offset,
                text: new,
            });
        }
    }
    if let Some(fm) = meta.frontmatter_position {
        let span = fm.start.offset..fm.end.offset.min(text.len());
        let yaml = &text[span.clone()];
        let mut seen: BTreeSet<usize> = BTreeSet::new();
        for link in &meta.frontmatter_links {
            let Some(target) = target_of(&link.link) else {
                continue;
            };
            let Some(new) = rebuild_wikilink(&link.original, &target) else {
                continue;
            };
            for (at, _) in yaml.match_indices(link.original.as_str()) {
                if seen.insert(at) {
                    edits.push(Edit {
                        start: span.start + at,
                        end: span.start + at + link.original.len(),
                        text: new.clone(),
                    });
                }
            }
        }
    }
    if edits.is_empty() {
        return None;
    }
    edits.sort_by_key(|e| e.start);
    edits.dedup_by_key(|e| e.start);
    let mut out = String::with_capacity(text.len() + 64);
    let mut at = 0;
    for edit in &edits {
        if edit.start < at {
            continue;
        }
        out.push_str(&text[at..edit.start]);
        out.push_str(&edit.text);
        at = edit.end;
    }
    out.push_str(&text[at..]);
    Some((out, edits.len()))
}

/// Rebuilds `[[body]]` or `![[body]]` for its new target; `None` when the
/// written path already is the generated one.
fn rebuild_wikilink(original: &str, target: &Target<'_>) -> Option<String> {
    let (prefix, inner) = if let Some(rest) = original.strip_prefix("![[") {
        ("![[", rest)
    } else {
        ("[[", original.strip_prefix("[[")?)
    };
    let inner = inner.strip_suffix("]]")?;
    let (path_part, alias) = match inner.find('|') {
        Some(i) => (&inner[..i], Some(&inner[i + 1..])),
        None => (inner, None),
    };
    // `[[Note\|alias]]` (in a table, or as Obsidian's alias form): keep the backslash.
    let (path_part, escaped) = match path_part.strip_suffix('\\') {
        Some(p) if alias.is_some() => (p, true),
        _ => (path_part, false),
    };
    let (written, subpath) = match path_part.find('#') {
        Some(i) => (path_part[..i].trim(), path_part[i..].trim_end()),
        None => (path_part.trim(), ""),
    };
    // `[[#Heading]]` points into the note itself and never names a file.
    if written.is_empty() || written == target.new_path {
        return None;
    }
    let mut out = String::with_capacity(original.len() + 16);
    out.push_str(prefix);
    out.push_str(&target.new_path);
    out.push_str(subpath);
    if let Some(alias) = alias {
        out.push_str(if escaped { "\\|" } else { "|" });
        let old_base = basename(target.old_target);
        if alias == old_base && alias != written {
            out.push_str(basename(target.new_target));
        } else {
            out.push_str(alias);
        }
    }
    out.push_str("]]");
    Some(out)
}

/// Rebuilds `[text](dest "title")` or `![alt](dest)` for its new target;
/// `None` for reference-style links and when nothing would change.
fn rebuild_markdown_link(original: &str, target: &Target<'_>) -> Option<String> {
    let (prefix, rest) = if let Some(r) = original.strip_prefix("![") {
        ("![", r)
    } else {
        ("[", original.strip_prefix('[')?)
    };
    let rest = rest.strip_suffix(')')?;
    let split = rest.rfind("](")?;
    let text = &rest[..split];
    let dest_and_tail = &rest[split + 2..];
    let lead_ws = dest_and_tail.len() - dest_and_tail.trim_start().len();
    let (lead, body) = dest_and_tail.split_at(lead_ws);
    let (angle, dest_raw, tail) = if let Some(b) = body.strip_prefix('<') {
        let close = b.find('>')?;
        (true, &b[..close], &b[close + 1..])
    } else {
        let end = body.find(char::is_whitespace).unwrap_or(body.len());
        (false, &body[..end], &body[end..])
    };
    let (dest_path, fragment) = match dest_raw.find('#') {
        Some(i) => (&dest_raw[..i], &dest_raw[i..]),
        None => (dest_raw, ""),
    };
    if dest_path.is_empty() {
        return None;
    }
    let new_dest_path = if FileKind::from_path(target.new_target).is_markdown() {
        format!("{}.md", target.new_path)
    } else {
        target.new_path.clone()
    };
    let new_dest = if angle {
        new_dest_path.clone()
    } else {
        encode_destination(&new_dest_path)
    };
    // The written destination without percent-encoding and extension.
    let written_decoded = crate::percent_decode(dest_path);
    let written_path = written_decoded
        .strip_suffix(".md")
        .unwrap_or(&written_decoded);
    let old_base = basename(target.old_target);
    let new_text = if text == old_base {
        basename(target.new_target)
    } else if text == written_path {
        target.new_path.as_str()
    } else {
        text
    };
    if dest_path == new_dest && new_text == text {
        return None;
    }
    let mut out = String::with_capacity(original.len() + 16);
    out.push_str(prefix);
    out.push_str(new_text);
    out.push_str("](");
    out.push_str(lead);
    if angle {
        out.push('<');
    }
    out.push_str(&new_dest);
    out.push_str(fragment);
    if angle {
        out.push('>');
    }
    out.push_str(tail);
    out.push(')');
    Some(out)
}

impl MetadataCache {
    /// Plans every note rewrite a rename batch needs: notes linking to a
    /// renamed file, and the renamed notes themselves (for links that would
    /// break). `texts` returns a note's current text by its old path.
    pub fn plan_rename_rewrites(
        &self,
        renames: &[(String, String)],
        format: LinkFormat,
        texts: &dyn Fn(&str) -> Option<String>,
    ) -> Vec<Rewrite> {
        let map: HashMap<String, String> = renames.iter().cloned().collect();
        let before = self.resolver();
        let mut after = before.clone();
        for (old, _) in renames {
            after.remove(old);
        }
        for (_, new) in renames {
            after.insert(new);
        }
        let mut candidates: BTreeSet<String> = BTreeSet::new();
        for (old, _) in renames {
            candidates.extend(self.sources_linking_to(old).map(str::to_string));
            if self.file(old).is_some() {
                candidates.insert(old.clone());
            }
        }
        let mut out = Vec::new();
        for old_path in candidates {
            let Some(meta) = self.file(&old_path) else {
                continue;
            };
            let Some(text) = texts(&old_path) else {
                continue;
            };
            let new_path = map
                .get(&old_path)
                .cloned()
                .unwrap_or_else(|| old_path.clone());
            if let Some((text, links)) = rewrite_note(
                &text, meta, &old_path, &new_path, &map, before, &after, format,
            ) {
                out.push(Rewrite {
                    old_path,
                    new_path,
                    text,
                    links,
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index_markdown;

    const A: &str = "# Top\n\npara ^blk\n\nself [[A#Top]] [[#Top]] [[A]] rel [S](Sub/S.md) [[Sub/S]] [[S]] ext [x](https://e.com/A.md)\n\n![[pic.png]] [[Dup]] [[Other/Dup]]\n";
    const B: &str = "---\nrelated: \"[[A]]\"\nlist:\n  - \"[[A|x]]\"\n  - \"[[A#Top]]\"\n---\n[[A]] [[a]] [[A|alias]] [[A#Top]] [[A#^blk]] [[A#Top|al]] ![[A]] ![[A#Top]] [[A.md]] [[A | spaced]]\n\n[A](A.md) [text](A.md#Top) [t](<A.md>) ![[A|300]] ![i](A.md) [A](./A.md) [A](/A.md)\n\n`[[A]]` and\n\n```\n[[A]]\n```\n\n[[Dup]] [[Other/Dup]] [[Sub/S]] [[S]] [[T]] [[Sub/T|tt]] [S](Sub/S.md) [[pic.png]] ![[pic.png]] [[Missing]]\n";
    const B2: &str = "---\naliases: [Bee]\nrelated: \"[[A]]\"\n---\n[[A|A]] [[A|a]] [[A#Top|A]] [[A#Top|Top]] [[A|A.md]] [[ A ]] [[A#Top |x]] [[A#Top| x ]] [[Sub/S|Sub/S]] [[Sub/S|S]] [[S|S]] [[S|Sub/S]]\n\n[A.md](A.md) [a](A.md) [A](A.md \"title\") [A][ref] [A] ![[A#Top|A]] [S](Sub/S.md) [Sub/S](Sub/S.md) [S.md](Sub/S.md) [Sub/S.md](Sub/S.md)\n\n> [!note] [[A]] in callout\n> [[A]]\n\n| a | b |\n| - | - |\n| [[A]] | [[A\\|x]] |\n\n- [ ] [[A]] task\n\n[ref]: A.md\n[A]: A.md\n";
    const S: &str = "[[A]] [../A](../A.md) [[../A]] [[T]] [T](T.md) [[Sub/T]] [[B]] [B](../B.md) [[Dup]] [[Other/Dup]]\n";
    const T: &str = "[[S]] [[Sub/S]] [S](S.md) [[A]]\n";
    const C: &str = "[[Dup]] [[Other/Dup]] [[A]] [[S]]\n";

    fn vault() -> (MetadataCache, HashMap<String, String>) {
        let notes = [
            ("A.md", A),
            ("B.md", B),
            ("B2.md", B2),
            ("Sub/S.md", S),
            ("Sub/T.md", T),
            ("Dup.md", "root dup [[Other/Dup]]\n"),
            ("Other/Dup.md", "other dup [[Dup]] [[Other/Dup]]\n"),
            ("Other/C.md", C),
        ];
        let mut paths: Vec<&str> = notes.iter().map(|(p, _)| *p).collect();
        paths.push("pic.png");
        let cache = MetadataCache::new(
            LinkResolver::new(paths),
            notes.iter().map(|(p, t)| (*p, index_markdown(t))),
        );
        let texts = notes
            .iter()
            .map(|(p, t)| (p.to_string(), t.to_string()))
            .collect();
        (cache, texts)
    }

    fn run(renames: &[(&str, &str)], format: LinkFormat) -> HashMap<String, String> {
        let (cache, texts) = vault();
        let renames: Vec<(String, String)> = renames
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        let plan = cache.plan_rename_rewrites(&renames, format, &|p| texts.get(p).cloned());
        plan.into_iter().map(|r| (r.new_path, r.text)).collect()
    }

    #[test]
    fn shortest_rename_regenerates_every_link_form() {
        let out = run(&[("A.md", "Renamed A.md")], LinkFormat::Shortest);
        assert_eq!(
            out["B.md"],
            "---\nrelated: \"[[Renamed A]]\"\nlist:\n  - \"[[Renamed A|x]]\"\n  - \"[[Renamed A#Top]]\"\n---\n[[Renamed A]] [[Renamed A]] [[Renamed A|alias]] [[Renamed A#Top]] [[Renamed A#^blk]] [[Renamed A#Top|al]] ![[Renamed A]] ![[Renamed A#Top]] [[Renamed A]] [[Renamed A| spaced]]\n\n[Renamed A](Renamed%20A.md) [text](Renamed%20A.md#Top) [t](<Renamed A.md>) ![[Renamed A|300]] ![i](Renamed%20A.md) [Renamed A](Renamed%20A.md) [Renamed A](Renamed%20A.md)\n\n`[[A]]` and\n\n```\n[[A]]\n```\n\n[[Dup]] [[Other/Dup]] [[Sub/S]] [[S]] [[T]] [[Sub/T|tt]] [S](Sub/S.md) [[pic.png]] ![[pic.png]] [[Missing]]\n"
        );
        assert_eq!(
            out["Renamed A.md"],
            "# Top\n\npara ^blk\n\nself [[Renamed A#Top]] [[#Top]] [[Renamed A]] rel [S](Sub/S.md) [[Sub/S]] [[S]] ext [x](https://e.com/A.md)\n\n![[pic.png]] [[Dup]] [[Other/Dup]]\n"
        );
        assert_eq!(
            out["Sub/S.md"],
            "[[Renamed A]] [Renamed A](Renamed%20A.md) [[Renamed A]] [[T]] [T](T.md) [[Sub/T]] [[B]] [B](../B.md) [[Dup]] [[Other/Dup]]\n"
        );
        assert_eq!(out["Sub/T.md"], "[[S]] [[Sub/S]] [S](S.md) [[Renamed A]]\n");
        assert_eq!(
            out["Other/C.md"],
            "[[Dup]] [[Other/Dup]] [[Renamed A]] [[S]]\n"
        );
        assert!(!out.contains_key("Dup.md"));
    }

    #[test]
    fn aliases_and_display_text_follow_the_observed_rules() {
        let out = run(&[("A.md", "A2.md")], LinkFormat::Shortest);
        assert_eq!(
            out["B2.md"],
            "---\naliases: [Bee]\nrelated: \"[[A2]]\"\n---\n[[A2|A]] [[A2|a]] [[A2#Top|A]] [[A2#Top|Top]] [[A2|A.md]] [[A2]] [[A2#Top|x]] [[A2#Top| x ]] [[Sub/S|Sub/S]] [[Sub/S|S]] [[S|S]] [[S|Sub/S]]\n\n[A.md](A2.md) [a](A2.md) [A2](A2.md \"title\") [A][ref] [A] ![[A2#Top|A]] [S](Sub/S.md) [Sub/S](Sub/S.md) [S.md](Sub/S.md) [Sub/S.md](Sub/S.md)\n\n> [!note] [[A2]] in callout\n> [[A2]]\n\n| a | b |\n| - | - |\n| [[A2]] | [[A2\\|x]] |\n\n- [ ] [[A2]] task\n\n[ref]: A.md\n[A]: A.md\n"
        );
        let out = run(&[("Sub/S.md", "Sub/S2.md")], LinkFormat::Shortest);
        assert_eq!(
            out["B2.md"].lines().nth(4).unwrap(),
            "[[A|A]] [[A|a]] [[A#Top|A]] [[A#Top|Top]] [[A|A.md]] [[ A ]] [[A#Top |x]] [[A#Top| x ]] [[S2|Sub/S]] [[S2|S2]] [[S2|S]] [[S2|Sub/S]]"
        );
        assert_eq!(
            out["B2.md"].lines().nth(6).unwrap(),
            "[A.md](A.md) [a](A.md) [A](A.md \"title\") [A][ref] [A] ![[A#Top|A]] [S2](S2.md) [S2](S2.md) [S.md](S2.md) [Sub/S.md](S2.md)"
        );
        assert_eq!(out["Sub/T.md"], "[[S2]] [[S2]] [S2](S2.md) [[A]]\n");
        assert_eq!(
            out["A.md"],
            "# Top\n\npara ^blk\n\nself [[A#Top]] [[#Top]] [[A]] rel [S2](S2.md) [[S2]] [[S2]] ext [x](https://e.com/A.md)\n\n![[pic.png]] [[Dup]] [[Other/Dup]]\n"
        );
    }

    #[test]
    fn moving_keeps_links_whose_text_is_unchanged() {
        let out = run(&[("A.md", "Moved/A.md")], LinkFormat::Shortest);
        // Only the forms whose written path differs from `A` are touched.
        assert_eq!(
            out["B.md"].lines().nth(6).unwrap(),
            "[[A]] [[A]] [[A|alias]] [[A#Top]] [[A#^blk]] [[A#Top|al]] ![[A]] ![[A#Top]] [[A]] [[A | spaced]]"
        );
        assert_eq!(
            out["B.md"].lines().nth(8).unwrap(),
            "[A](A.md) [text](A.md#Top) [t](<A.md>) ![[A|300]] ![i](A.md) [A](A.md) [A](A.md)"
        );
        assert_eq!(out["B.md"].lines().nth(1).unwrap(), "related: \"[[A]]\"");
        // The moved note's own links still resolve, so it is not rewritten.
        assert!(!out.contains_key("Moved/A.md"));
        assert!(!out.contains_key("Sub/T.md"));
    }

    #[test]
    fn a_moved_note_fixes_only_the_links_that_would_break() {
        let out = run(&[("B.md", "Other/B.md")], LinkFormat::Shortest);
        assert_eq!(
            out["Other/B.md"].lines().nth(8).unwrap(),
            "[A](A.md) [text](A.md#Top) [t](<A.md>) ![[A|300]] ![i](A.md) [A](A.md) [A](/A.md)"
        );
        assert_eq!(
            out["Other/B.md"].lines().nth(6).unwrap(),
            "[[A]] [[a]] [[A|alias]] [[A#Top]] [[A#^blk]] [[A#Top|al]] ![[A]] ![[A#Top]] [[A.md]] [[A | spaced]]"
        );
        assert_eq!(
            out["Sub/S.md"],
            "[[A]] [../A](../A.md) [[../A]] [[T]] [T](T.md) [[Sub/T]] [[B]] [B](B.md) [[Dup]] [[Other/Dup]]\n"
        );
    }

    #[test]
    fn ambiguous_basenames_fall_back_to_the_full_path() {
        let out = run(&[("Other/Dup.md", "Other/Dup2.md")], LinkFormat::Shortest);
        assert_eq!(out["Dup.md"], "root dup [[Dup2]]\n");
        assert_eq!(out["Other/Dup2.md"], "other dup [[Dup]] [[Dup2]]\n");
        assert_eq!(out["Other/C.md"], "[[Dup]] [[Dup2]] [[A]] [[S]]\n");
        let out = run(&[("A.md", "Sub/Dup.md")], LinkFormat::Shortest);
        assert_eq!(
            out["B.md"].lines().nth(6).unwrap(),
            "[[Sub/Dup]] [[Sub/Dup]] [[Sub/Dup|alias]] [[Sub/Dup#Top]] [[Sub/Dup#^blk]] [[Sub/Dup#Top|al]] ![[Sub/Dup]] ![[Sub/Dup#Top]] [[Sub/Dup]] [[Sub/Dup| spaced]]"
        );
        assert_eq!(
            out["B.md"].lines().nth(8).unwrap(),
            "[Dup](Sub/Dup.md) [text](Sub/Dup.md#Top) [t](<Sub/Dup.md>) ![[Sub/Dup|300]] ![i](Sub/Dup.md) [Dup](Sub/Dup.md) [Dup](Sub/Dup.md)"
        );
        assert_eq!(
            out["Sub/S.md"],
            "[[Sub/Dup]] [Sub/Dup](Sub/Dup.md) [[Sub/Dup]] [[T]] [T](T.md) [[Sub/T]] [[B]] [B](../B.md) [[Dup]] [[Other/Dup]]\n"
        );
    }

    #[test]
    fn folder_renames_are_a_batch_of_file_renames() {
        let out = run(
            &[("Sub/S.md", "Sub2/S.md"), ("Sub/T.md", "Sub2/T.md")],
            LinkFormat::Shortest,
        );
        assert_eq!(
            out["Sub2/S.md"],
            "[[A]] [../A](../A.md) [[../A]] [[T]] [T](T.md) [[T]] [[B]] [B](../B.md) [[Dup]] [[Other/Dup]]\n"
        );
        assert_eq!(out["Sub2/T.md"], "[[S]] [[S]] [S](S.md) [[A]]\n");
        assert_eq!(
            out["B.md"].lines().nth(16).unwrap(),
            "[[Dup]] [[Other/Dup]] [[S]] [[S]] [[T]] [[T|tt]] [S](S.md) [[pic.png]] ![[pic.png]] [[Missing]]"
        );
        let out = run(
            &[
                ("Sub/S.md", "Other/Sub/S.md"),
                ("Sub/T.md", "Other/Sub/T.md"),
            ],
            LinkFormat::Shortest,
        );
        assert_eq!(
            out["Other/Sub/S.md"],
            "[[A]] [A](A.md) [[A]] [[T]] [T](T.md) [[T]] [[B]] [B](B.md) [[Dup]] [[Other/Dup]]\n"
        );
    }

    #[test]
    fn attachments_keep_their_extension_and_case_only_renames_apply() {
        let out = run(&[("pic.png", "Other/pic2.png")], LinkFormat::Shortest);
        assert_eq!(
            out["B.md"].lines().nth(16).unwrap(),
            "[[Dup]] [[Other/Dup]] [[Sub/S]] [[S]] [[T]] [[Sub/T|tt]] [S](Sub/S.md) [[pic2.png]] ![[pic2.png]] [[Missing]]"
        );
        assert!(out["A.md"].contains("![[pic2.png]]"));
        let out = run(&[("A.md", "a.md")], LinkFormat::Shortest);
        assert_eq!(
            out["B.md"].lines().nth(8).unwrap(),
            "[a](a.md) [text](a.md#Top) [t](<a.md>) ![[a|300]] ![i](a.md) [a](a.md) [a](a.md)"
        );
        assert!(out["a.md"].contains("self [[a#Top]] [[#Top]] [[a]]"));
    }

    #[test]
    fn relative_and_absolute_formats() {
        let out = run(&[("A.md", "A2.md")], LinkFormat::Relative);
        assert_eq!(
            out["Sub/S.md"],
            "[[../A2]] [../A2](../A2.md) [[../A2]] [[T]] [T](T.md) [[Sub/T]] [[B]] [B](../B.md) [[Dup]] [[Other/Dup]]\n"
        );
        assert_eq!(out["Other/C.md"], "[[Dup]] [[Other/Dup]] [[../A2]] [[S]]\n");
        let out = run(&[("A.md", "Moved/A.md")], LinkFormat::Relative);
        assert_eq!(
            out["B.md"].lines().nth(8).unwrap(),
            "[A](Moved/A.md) [text](Moved/A.md#Top) [t](<Moved/A.md>) ![[Moved/A|300]] ![i](Moved/A.md) [A](Moved/A.md) [A](Moved/A.md)"
        );
        assert!(out["Sub/S.md"]
            .starts_with("[[../Moved/A]] [../Moved/A](../Moved/A.md) [[../Moved/A]]"));
        assert!(
            !out.contains_key("Moved/A.md"),
            "self links regenerate to the same text"
        );
        let out = run(&[("Sub/S.md", "Sub/S2.md")], LinkFormat::Relative);
        assert!(out["A.md"].contains("rel [S2](Sub/S2.md) [[Sub/S2]] [[Sub/S2]]"));
        assert_eq!(
            out["Other/C.md"],
            "[[Dup]] [[Other/Dup]] [[A]] [[../Sub/S2]]\n"
        );
        assert_eq!(out["Sub/T.md"], "[[S2]] [[S2]] [S2](S2.md) [[A]]\n");
        assert_eq!(
            out["B2.md"].lines().nth(4).unwrap(),
            "[[A|A]] [[A|a]] [[A#Top|A]] [[A#Top|Top]] [[A|A.md]] [[ A ]] [[A#Top |x]] [[A#Top| x ]] [[Sub/S2|Sub/S]] [[Sub/S2|S2]] [[Sub/S2|S]] [[Sub/S2|Sub/S]]"
        );
        assert_eq!(
            out["B2.md"].lines().nth(6).unwrap(),
            "[A.md](A.md) [a](A.md) [A](A.md \"title\") [A][ref] [A] ![[A#Top|A]] [S2](Sub/S2.md) [Sub/S2](Sub/S2.md) [S.md](Sub/S2.md) [Sub/S.md](Sub/S2.md)"
        );
        let out = run(&[("Sub/S.md", "Sub/S2.md")], LinkFormat::Absolute);
        assert_eq!(
            out["Sub/T.md"],
            "[[Sub/S2]] [[Sub/S2]] [S2](Sub/S2.md) [[A]]\n"
        );
        assert_eq!(
            out["Other/C.md"],
            "[[Dup]] [[Other/Dup]] [[A]] [[Sub/S2]]\n"
        );
    }

    #[test]
    fn special_characters_encode_only_spaces() {
        let out = run(&[("A.md", "Ä (b) 100%.md")], LinkFormat::Shortest);
        assert_eq!(
            out["B.md"].lines().nth(8).unwrap(),
            "[Ä (b) 100%](Ä%20(b)%20100%.md) [text](Ä%20(b)%20100%.md#Top) [t](<Ä (b) 100%.md>) ![[Ä (b) 100%|300]] ![i](Ä%20(b)%20100%.md) [Ä (b) 100%](Ä%20(b)%20100%.md) [Ä (b) 100%](Ä%20(b)%20100%.md)"
        );
        assert_eq!(
            out["B.md"].lines().nth(1).unwrap(),
            "related: \"[[Ä (b) 100%]]\""
        );
    }

    #[test]
    fn link_paths_per_format() {
        let r = LinkResolver::new(["A.md", "Sub/S.md", "Dup.md", "Other/Dup.md", "pic.png"]);
        assert_eq!(link_path(&r, "A.md", "Sub/S.md", LinkFormat::Shortest), "A");
        assert_eq!(
            link_path(&r, "A.md", "Sub/S.md", LinkFormat::Relative),
            "../A"
        );
        assert_eq!(
            link_path(&r, "Sub/S.md", "A.md", LinkFormat::Relative),
            "Sub/S"
        );
        assert_eq!(
            link_path(&r, "Sub/S.md", "Sub/T.md", LinkFormat::Relative),
            "S"
        );
        assert_eq!(
            link_path(&r, "Sub/S.md", "A.md", LinkFormat::Absolute),
            "Sub/S"
        );
        assert_eq!(
            link_path(&r, "Other/Dup.md", "A.md", LinkFormat::Shortest),
            "Other/Dup"
        );
        assert_eq!(
            link_path(&r, "Other/Dup.md", "Other/C.md", LinkFormat::Shortest),
            "Other/Dup",
            "`Dup` from Other/C.md resolves to the root file, so the path is needed"
        );
        assert_eq!(
            link_path(&r, "pic.png", "Sub/S.md", LinkFormat::Shortest),
            "pic.png"
        );
        assert_eq!(
            LinkFormat::from_config(Some("relative")),
            LinkFormat::Relative
        );
        assert_eq!(LinkFormat::from_config(None), LinkFormat::Shortest);
        assert_eq!(basename("Sub/Note v1.2.md"), "Note v1.2");
        assert_eq!(basename("pic.png"), "pic");
    }
}
