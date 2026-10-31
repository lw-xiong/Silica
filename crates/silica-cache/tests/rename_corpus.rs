//! Renames over the conformance corpus: every link that resolved before a
//! batch of renames must resolve to the same (renamed) file afterwards, no
//! link may appear or vanish, and nothing outside the links may change.
//! The corpus is `silica-markdown`'s 1000-note vault plus its edge-case
//! notes, so the rewriter meets every construct the parser knows.

use silica_cache::{index_markdown, LinkFormat, LinkResolver, MetadataCache};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

fn corpus_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../silica-markdown/tests/corpus/vault")
}

fn load() -> (Vec<String>, BTreeMap<String, String>) {
    let root = corpus_root();
    let mut paths = Vec::new();
    let mut texts = BTreeMap::new();
    for entry in walkdir(&root) {
        let rel = entry
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if rel.split('/').any(|p| p.starts_with('.')) {
            continue;
        }
        if rel.ends_with(".md") {
            texts.insert(rel.clone(), std::fs::read_to_string(&entry).unwrap());
        }
        paths.push(rel);
    }
    (paths, texts)
}

fn walkdir(root: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out
}

fn build(paths: &[String], texts: &BTreeMap<String, String>) -> MetadataCache {
    MetadataCache::new(
        LinkResolver::new(paths.iter().map(String::as_str)),
        texts.iter().map(|(p, t)| (p.clone(), index_markdown(t))),
    )
}

/// Every resolved link edge as (source, target, count), plus the number of
/// unresolved links per source.
fn edges(cache: &MetadataCache) -> (BTreeMap<(String, String), usize>, BTreeMap<String, usize>) {
    let mut resolved = BTreeMap::new();
    let mut unresolved = BTreeMap::new();
    for (path, _) in cache.files() {
        if let Some(targets) = cache.resolved_links(path) {
            for (target, n) in targets {
                resolved.insert((path.to_string(), target.clone()), *n);
            }
        }
        if let Some(links) = cache.unresolved_links(path) {
            unresolved.insert(path.to_string(), links.values().sum());
        }
    }
    (resolved, unresolved)
}

/// The text with every link and embed occurrence blanked, to compare what
/// a rewrite must leave alone.
fn without_links(text: &str) -> String {
    let meta = index_markdown(text);
    let mut spans: Vec<(usize, usize)> = meta
        .links
        .iter()
        .chain(meta.embeds.iter())
        .map(|l| (l.position.start.offset, l.position.end.offset))
        .collect();
    if let Some(fm) = meta.frontmatter_position {
        spans.push((fm.start.offset, fm.end.offset.min(text.len())));
    }
    spans.sort();
    let mut out = String::new();
    let mut at = 0;
    for (s, e) in spans {
        if s < at {
            continue;
        }
        out.push_str(&text[at..s]);
        out.push('\u{1}');
        at = e;
    }
    out.push_str(&text[at..]);
    out
}

fn check(renames: &[(String, String)], format: LinkFormat) {
    let (paths, texts) = load();
    let before = build(&paths, &texts);
    let (resolved_before, unresolved_before) = edges(&before);
    let map: HashMap<&str, &str> = renames
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let rewrites = before.plan_rename_rewrites(renames, format, &|p| texts.get(p).cloned());

    // Apply the renames and rewrites to a copy of the vault.
    let new_paths: Vec<String> = paths
        .iter()
        .map(|p| {
            map.get(p.as_str())
                .map(|s| s.to_string())
                .unwrap_or(p.clone())
        })
        .collect();
    let mut new_texts: BTreeMap<String, String> = texts
        .iter()
        .map(|(p, t)| {
            (
                map.get(p.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or(p.clone()),
                t.clone(),
            )
        })
        .collect();
    for rw in &rewrites {
        assert_eq!(
            without_links(&rw.text),
            without_links(&texts[&rw.old_path]),
            "{}: text outside links changed",
            rw.old_path
        );
        new_texts.insert(rw.new_path.clone(), rw.text.clone());
    }
    let after = build(&new_paths, &new_texts);
    let (resolved_after, unresolved_after) = edges(&after);

    let rename = |p: &str| map.get(p).map(|s| s.to_string()).unwrap_or(p.to_string());
    let expected: BTreeMap<(String, String), usize> = resolved_before
        .iter()
        .map(|((s, t), n)| ((rename(s), rename(t)), *n))
        .collect();
    let expected_unresolved: BTreeMap<String, usize> = unresolved_before
        .iter()
        .map(|(s, n)| (rename(s), *n))
        .collect();
    let missing: Vec<_> = expected
        .iter()
        .filter(|(k, n)| resolved_after.get(*k) != Some(*n))
        .take(10)
        .collect();
    assert!(missing.is_empty(), "edges lost or changed: {missing:?}");
    let extra: Vec<_> = resolved_after
        .iter()
        .filter(|(k, _)| !expected.contains_key(*k))
        .take(10)
        .collect();
    assert!(extra.is_empty(), "edges appeared: {extra:?}");
    assert_eq!(
        unresolved_after, expected_unresolved,
        "unresolved links changed"
    );
    assert!(!rewrites.is_empty(), "the renames touched no link");
}

#[test]
fn renaming_every_note_keeps_every_link() {
    let (paths, _) = load();
    assert!(paths.len() > 1000, "corpus present");
    let renames: Vec<(String, String)> = paths
        .iter()
        .filter(|p| p.ends_with(".md"))
        .map(|p| {
            let stem = p.strip_suffix(".md").unwrap();
            (p.clone(), format!("{stem} renamed.md"))
        })
        .collect();
    check(&renames, LinkFormat::Shortest);
}

#[test]
fn moving_folders_keeps_every_link_in_every_format() {
    let (paths, _) = load();
    let mut folders: Vec<String> = paths
        .iter()
        .filter_map(|p| p.rsplit_once('/').map(|(d, _)| d.to_string()))
        .collect();
    folders.sort();
    folders.dedup();
    // Move the first three folders under a new one and rename the fourth.
    let mut renames = Vec::new();
    for (i, folder) in folders.iter().take(4).enumerate() {
        let target = if i < 3 {
            format!("Archive/{folder}")
        } else {
            format!("{folder} moved")
        };
        let prefix = format!("{folder}/");
        for p in paths.iter().filter(|p| p.starts_with(&prefix)) {
            renames.push((p.clone(), format!("{target}/{}", &p[prefix.len()..])));
        }
    }
    assert!(renames.len() > 10, "folders found: {folders:?}");
    for format in [
        LinkFormat::Shortest,
        LinkFormat::Relative,
        LinkFormat::Absolute,
    ] {
        check(&renames, format);
    }
}

#[test]
fn attachments_rename_too() {
    let (paths, _) = load();
    let renames: Vec<(String, String)> = paths
        .iter()
        .filter(|p| !p.ends_with(".md"))
        .map(|p| {
            (
                p.clone(),
                format!("Attachments/moved {}", p.rsplit('/').next().unwrap()),
            )
        })
        .collect();
    if renames.is_empty() {
        return;
    }
    check(&renames, LinkFormat::Shortest);
}
