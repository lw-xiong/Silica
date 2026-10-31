//! Link resolution with Obsidian's rules (`getFirstLinkpathDest`).
//!
//! 1. Strip `#subpath` and `|alias`.
//! 2. With a known extension, look for that exact file; without one, try `.md`
//!    then every other known extension.
//! 3. A linkpath starting with `/` resolves from the vault root only; one
//!    starting with `./` or `../` resolves relative to the source file's
//!    folder only (observed: `[A](./A.md)` breaks once the source moves into
//!    a subfolder, `[A](/A.md)` does not); any other linkpath containing `/`
//!    resolves from the vault root, then relative to the source's folder.
//! 4. Otherwise look up the basename: a file at the vault root wins, then
//!    one in the source's own folder, then the shortest path (in characters),
//!    then path order. Observed in Obsidian 1.14.4 from `Long/Way/Down/C.md`:
//!    `[[Q]]` is `Q.md` over `Long/Way/Down/Q.md`, `[[X]]` is
//!    `Long/Way/Down/X.md` over `Mid/X.md`, `[[W]]` is `Long/Way/W.md` over
//!    `Aaaa/Bbbb/W.md`, `[[N]]` is `Aaaa/N.md` over `Zzzz/N.md`.
//! 5. Everything is case-insensitive; `\` is normalized to `/`.

use silica_formats::{link_resolution_extensions, FileKind};
use std::collections::{BTreeSet, HashMap};

#[derive(Debug, Default, Clone)]
pub struct LinkResolver {
    /// Original-case vault-relative paths.
    files: BTreeSet<String>,
    /// Lowercased path -> original path.
    by_lower: HashMap<String, String>,
    /// Lowercased basename (without extension) -> original paths.
    by_basename: HashMap<String, Vec<String>>,
}

fn normalize(path: &str) -> String {
    path.replace('\\', "/").trim_matches('/').to_string()
}

fn basename_no_ext(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => name,
    }
}

/// The folder part of a vault path (`""` at the root).
pub fn folder_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(i) => &path[..i],
        None => "",
    }
}

/// Joins `rel` onto `folder`, resolving `.` and `..` segments; `None` when
/// `..` would leave the vault.
fn join_relative(folder: &str, rel: &str) -> Option<String> {
    let mut parts: Vec<&str> = folder.split('/').filter(|p| !p.is_empty()).collect();
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

impl LinkResolver {
    pub fn new<I, S>(paths: I) -> LinkResolver
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut r = LinkResolver::default();
        for p in paths {
            r.insert(p.as_ref());
        }
        r
    }

    pub fn insert(&mut self, path: &str) {
        let path = normalize(path);
        if !self.files.insert(path.clone()) {
            return;
        }
        self.by_lower.insert(path.to_lowercase(), path.clone());
        self.by_basename
            .entry(basename_no_ext(&path).to_lowercase())
            .or_default()
            .push(path);
    }

    pub fn remove(&mut self, path: &str) {
        let path = normalize(path);
        if !self.files.remove(&path) {
            return;
        }
        self.by_lower.remove(&path.to_lowercase());
        if let Some(v) = self
            .by_basename
            .get_mut(&basename_no_ext(&path).to_lowercase())
        {
            v.retain(|p| p != &path);
        }
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    fn exact(&self, candidate: &str) -> Option<&str> {
        self.by_lower
            .get(&candidate.to_lowercase())
            .map(String::as_str)
    }

    fn with_extensions(&self, stem: &str) -> Option<&str> {
        if FileKind::from_path(stem) != FileKind::Other {
            if let Some(p) = self.exact(stem) {
                return Some(p);
            }
        }
        link_resolution_extensions().find_map(|ext| self.exact(&format!("{stem}.{ext}")))
    }

    /// Resolves the body of a wikilink (alias and subpath allowed) from `source_path`.
    pub fn resolve(&self, linktext: &str, source_path: &str) -> Option<&str> {
        let target = silica_markdown::parse_linktext(linktext).path;
        if target.is_empty() {
            return self.exact(&normalize(source_path));
        }
        let raw = target.replace('\\', "/");
        if let Some(abs) = raw.strip_prefix('/') {
            return self.with_extensions(&normalize(abs));
        }
        if raw.starts_with("./") || raw.starts_with("../") || raw == "." || raw == ".." {
            let source_norm = normalize(source_path);
            let joined = join_relative(folder_of(&source_norm), &raw)?;
            return self.with_extensions(&joined);
        }
        let target = normalize(target);
        if target.contains('/') {
            if let Some(p) = self.with_extensions(&target) {
                return Some(p);
            }
            let source_norm = normalize(source_path);
            let rel = join_relative(folder_of(&source_norm), &target)?;
            return self.with_extensions(&rel);
        }
        let candidates = self
            .by_basename
            .get(&basename_no_ext(&target).to_lowercase())?;
        let want_ext = target.rsplit_once('.').map(|(_, e)| e.to_lowercase());
        let matching: Vec<&String> = candidates
            .iter()
            .filter(|p| match &want_ext {
                Some(ext) if FileKind::from_extension(ext) != FileKind::Other => {
                    p.to_lowercase().ends_with(&format!(".{ext}"))
                }
                _ => true,
            })
            .collect();
        if matching.is_empty() {
            return None;
        }
        let source_folder = folder_of(&normalize(source_path)).to_lowercase();
        // Prefer markdown when the link has no extension, then the vault
        // root, then the source's folder, then the shortest path.
        let score = |p: &&String| {
            let md = if want_ext.is_none() && FileKind::from_path(p).is_markdown() {
                0
            } else {
                1
            };
            let folder = folder_of(p).to_lowercase();
            let root = if folder.is_empty() { 0 } else { 1 };
            let same = if folder == source_folder { 0 } else { 1 };
            (md, root, same, p.len(), p.to_lowercase())
        };
        matching.into_iter().min_by_key(score).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolver() -> LinkResolver {
        LinkResolver::new([
            "Note.md",
            "Projects/Note.md",
            "Projects/Plan.md",
            "Deep/Nested/Plan.md",
            "Assets/pic.png",
            "Board.canvas",
            "Projects/Note.canvas",
        ])
    }

    #[test]
    fn shortest_path_wins_then_same_folder() {
        let r = resolver();
        assert_eq!(r.resolve("Note", "Other.md"), Some("Note.md"));
        assert_eq!(
            r.resolve("Note", "Projects/Plan.md"),
            Some("Note.md"),
            "the shorter root path beats the same folder"
        );
        assert_eq!(
            r.resolve("Plan", "Deep/Nested/X.md"),
            Some("Deep/Nested/Plan.md"),
            "the source's folder beats a shorter path elsewhere"
        );
        assert_eq!(r.resolve("Plan", "Other.md"), Some("Projects/Plan.md"));
        // Observed in Obsidian 1.14.4 (see the module docs).
        let r = LinkResolver::new([
            "Aa/Dup.md",
            "Bb/Dup.md",
            "Zz/Dup.md",
            "Zed.md",
            "Long/Way/Down/Zed.md",
            "Long/Deep.md",
            "Long/Way/Deep.md",
            "Mid/X.md",
            "Long/Way/Down/X.md",
            "Other/Z.md",
            "Long/Way/Z.md",
            "Aaaa/Bbbb/W.md",
            "Long/Way/W.md",
            "P.md",
            "Long/Way/Down/P.png",
        ]);
        let deep = "Long/Way/Down/C.md";
        assert_eq!(r.resolve("Dup", "Bb/C.md"), Some("Bb/Dup.md"), "own folder");
        assert_eq!(r.resolve("Dup", "Root.md"), Some("Aa/Dup.md"), "path order");
        assert_eq!(
            r.resolve("Zed", deep),
            Some("Zed.md"),
            "root beats own folder"
        );
        assert_eq!(
            r.resolve("Deep", deep),
            Some("Long/Deep.md"),
            "shortest elsewhere"
        );
        assert_eq!(
            r.resolve("Deep", "Long/Way/Deep.md"),
            Some("Long/Way/Deep.md")
        );
        assert_eq!(
            r.resolve("X", deep),
            Some("Long/Way/Down/X.md"),
            "own folder beats shorter"
        );
        assert_eq!(
            r.resolve("Z", deep),
            Some("Other/Z.md"),
            "shorter beats the parent folder"
        );
        assert_eq!(
            r.resolve("W", deep),
            Some("Long/Way/W.md"),
            "length in characters"
        );
        assert_eq!(r.resolve("P", deep), Some("P.md"), "markdown first");
        assert_eq!(r.resolve("P.png", deep), Some("Long/Way/Down/P.png"));
    }

    #[test]
    fn case_insensitive_and_extension_aware() {
        let r = resolver();
        assert_eq!(r.resolve("note#Heading|alias", "x.md"), Some("Note.md"));
        assert_eq!(r.resolve("PIC.PNG", "x.md"), Some("Assets/pic.png"));
        assert_eq!(r.resolve("Board.canvas", "x.md"), Some("Board.canvas"));
        assert_eq!(
            r.resolve("Note.canvas", "x.md"),
            Some("Projects/Note.canvas")
        );
        assert_eq!(r.resolve("Missing", "x.md"), None);
    }

    #[test]
    fn paths_resolve_from_root_then_relative() {
        let r = resolver();
        assert_eq!(r.resolve("projects/note", "x.md"), Some("Projects/Note.md"));
        assert_eq!(
            r.resolve("Nested/Plan", "Deep/Other.md"),
            Some("Deep/Nested/Plan.md")
        );
        assert_eq!(
            r.resolve("#Heading", "Projects/Plan.md"),
            Some("Projects/Plan.md")
        );
    }

    #[test]
    fn dot_segments_are_relative_only_and_a_leading_slash_is_the_root() {
        let r = resolver();
        assert_eq!(r.resolve("./Note", "Other.md"), Some("Note.md"));
        assert_eq!(
            r.resolve("./Note.md", "Projects/Plan.md"),
            Some("Projects/Note.md")
        );
        assert_eq!(r.resolve("../Note", "Projects/Plan.md"), Some("Note.md"));
        assert_eq!(
            r.resolve("../Note", "Other.md"),
            None,
            "cannot leave the vault"
        );
        assert_eq!(r.resolve("/Note.md", "Projects/Plan.md"), Some("Note.md"));
        assert_eq!(
            r.resolve("/Plan", "Projects/Plan.md"),
            None,
            "no basename lookup from `/`"
        );
        assert_eq!(
            r.resolve("../Nested/Plan", "Deep/Other/X.md"),
            Some("Deep/Nested/Plan.md")
        );
    }
}
