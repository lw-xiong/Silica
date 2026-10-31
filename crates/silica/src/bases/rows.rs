//! The bridge: the vault's index and cache as the values a base evaluates.
//!
//! `silica-bases` reads no vault — it takes one [`FileData`] per note, with
//! the note's frontmatter already typed as [`Value`]s. This builds those from
//! [`VaultState`], and it is the only place that knows both sides.
//!
//! Everything here comes out of the **cache**: the file row the vault scan
//! already holds (path, size, ctime, mtime) and the `CachedMetadata` the
//! indexer already built (frontmatter, links, embeds, tags), plus the
//! vault-wide backlink index. Nothing in this module opens a file, so
//! rebuilding every row of a 20k-note vault is a walk over two in-memory
//! indexes rather than 20k reads.
//!
//! The field rules are observed; see
//! [[Specification#Observed Bases expression language, Obsidian 1.14.5 (installer build 1.13.7)]]
//! and the section after it. The ones that are easy to get wrong:
//!
//! - `file.name` drops a `.md` and keeps any other extension (`Silica`, but
//!   `Main.base`); `basename` is always the stem; `fullname` keeps the
//!   extension whatever it is.
//! - **`file.folder` for a file in the vault root is `/`**, not `""`.
//! - `file.tags` carry their `#`, frontmatter first, then the inline ones,
//!   merged case-insensitively — tags are the one case-insensitive value in
//!   the language.
//! - `file.links` is the note's frontmatter links *then* its body links, with
//!   the **written** target as the path; `file.backlinks` holds the *linking
//!   file's* full path (`[[Pumice.md|Pumice]]`), one entry per source file.
//! - `ctime`/`mtime` are civil date-times: the filesystem stamp read in the
//!   machine's own zone, with no zone carried, which is what Obsidian shows.

use crate::workspace::VaultState;
use indexmap::IndexMap;
use silica_bases::funcs::time::parse_date;
use silica_bases::{DateValue, Env, FileData, LinkValue, Table, Value};
use silica_cache::{all_tags, CachedMetadata, LinkCache, MetadataCache};
use silica_formats::base::Base;
use silica_formats::config::PropertyTypes;
use silica_formats::properties::{reserved_type, PropertyType};
use silica_formats::FileKind;
use silica_vault::VaultFile;
use std::collections::HashSet;
use std::sync::Arc;

/// A `.base` resolved against the vault: the parsed file, the table the
/// showing view resolves to, and the message a parse failure answered.
///
/// Both the view and an embedded `![[x.base]]` go through
/// [`resolve`], so the two cannot disagree about what a file means.
pub struct Resolved {
    /// `None` only when the file failed to parse, in which case nothing is
    /// drawn but the banner — and nothing is ever written back.
    pub base: Option<Base>,
    pub table: Option<Table>,
    pub error: Option<String>,
}

/// Parses `text` and resolves its `view_ix`th view against the vault.
///
/// With no vault — a test, or a view that has not been handed one yet — the
/// file still parses and the table comes out empty: the engine reads its rows
/// from [`Env::files`], and an empty one is a vault with no notes.
pub fn resolve(
    state: Option<&VaultState>,
    path: Option<&str>,
    text: &str,
    view_ix: usize,
) -> Resolved {
    match Base::from_yaml(text) {
        Err(e) => Resolved {
            base: None,
            table: None,
            error: Some(e.to_string()),
        },
        Ok(base) => {
            let env = match (state, path) {
                (Some(state), Some(path)) => env_for(state, path),
                _ => Env {
                    files: Vec::new(),
                    base_file: None,
                    now: chrono::Local::now().naive_local(),
                },
            };
            let table = silica_bases::table::build(&base, view_ix, &env);
            Resolved {
                base: Some(base),
                table: Some(table),
                error: None,
            }
        }
    }
}

/// Every note of the vault as a row candidate, with the base file's own
/// [`FileData`] for `this`.
pub fn env_for(state: &VaultState, base_path: &str) -> Env {
    let vault = state.vault();
    let cache = state.cache();
    let types = state.property_types();
    // Rows are notes. A `.base` draws a table of markdown files, and the
    // vault's attachments are not candidates for one.
    let files: Vec<Arc<FileData>> = vault
        .markdown_files()
        .map(|file| Arc::new(build(file, &cache, &types)))
        .collect();
    // The base file is not a note, so it is never among the rows; `this`
    // still has to answer `Main.base` for it. A base the vault has not
    // indexed yet -- one created a moment ago -- still gets its name.
    let base_file = Some(file_data(state, base_path).unwrap_or_else(|| Arc::new(bare(base_path))));
    Env {
        files,
        base_file,
        now: chrono::Local::now().naive_local(),
    }
}

/// One file of the vault as a [`FileData`], by its vault-relative path.
/// Paths compare case-insensitively, as everywhere else in the vault.
pub fn file_data(state: &VaultState, path: &str) -> Option<Arc<FileData>> {
    let vault = state.vault();
    let cache = state.cache();
    let types = state.property_types();
    let file = vault
        .files()
        .iter()
        .find(|f| f.path.eq_ignore_ascii_case(path))?;
    Some(Arc::new(build(file, &cache, &types)))
}

/// One file's values, from the scan row and the cache record alone.
fn build(file: &VaultFile, cache: &MetadataCache, types: &PropertyTypes) -> FileData {
    let path = file.path.as_str();
    let mut data = names(path);
    // The vault scan already stat'ed every file, so `size` is free here; it
    // is the one field that would otherwise cost a read per row.
    data.size = file.size as f64;
    data.ctime = civil(file.ctime);
    data.mtime = civil(file.mtime);
    data.backlinks = cache
        .sources_linking_to(path)
        .map(|source| LinkValue {
            path: source.to_string(),
            display: Some(names(source).name),
        })
        .collect();
    if let Some(meta) = cache.file(path) {
        data.tags = tags_of(meta);
        data.links = links_of(meta);
        data.embeds = meta.embeds.iter().map(link_value).collect();
        data.properties = properties_of(meta, types);
    }
    data
}

/// A file Silica has no scan row for: its names, and nothing else. A base
/// opened before the watcher caught the file still has to answer `this`.
fn bare(path: &str) -> FileData {
    names(path)
}

/// The five name fields. `name` drops a `.md` and keeps every other
/// extension; `basename` is always the stem; `fullname` always keeps it.
fn names(path: &str) -> FileData {
    let fullname = path.rsplit('/').next().unwrap_or(path).to_string();
    let ext = match fullname.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => ext.to_string(),
        _ => String::new(),
    };
    let basename = match fullname.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_string(),
        _ => fullname.clone(),
    };
    let name = if FileKind::from_path(path).is_markdown() {
        basename.clone()
    } else {
        fullname.clone()
    };
    FileData {
        name,
        basename,
        fullname,
        path: path.to_string(),
        folder: folder_of(path),
        ext,
        ..FileData::default()
    }
}

/// The folder a file sits in. **A file in the vault root is in `/`** — the
/// one folder value that is not a vault-relative path.
fn folder_of(path: &str) -> String {
    match path.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(cut) => path[..cut].to_string(),
    }
}

/// A filesystem stamp as a civil date-time. The stamp is UTC milliseconds and
/// Obsidian renders it in the machine's own zone, so the zone is applied here
/// and then dropped: nothing below this line knows one.
fn civil(ms: u64) -> Option<DateValue> {
    use chrono::TimeZone;
    match chrono::Local.timestamp_millis_opt(ms as i64) {
        chrono::offset::LocalResult::Single(at) => Some(DateValue {
            at: at.naive_local(),
            has_time: true,
        }),
        _ => None,
    }
}

/// The note's tags with their `#`, frontmatter first and then the inline
/// ones, merged case-insensitively. `all_tags` counts occurrences and so
/// repeats a tag written twice; a value does not.
fn tags_of(meta: &CachedMetadata) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    all_tags(meta)
        .into_iter()
        .filter(|tag| seen.insert(tag.to_lowercase()))
        .map(|tag| format!("#{tag}"))
        .collect()
}

/// Outgoing links: the frontmatter's first, then the body's, which is the
/// order observed (`linkish: "[[Pumice]]"` came before the body's `[[Pumice]]`
/// and `[[Basalt]]`). Embeds are their own list and are not in this one.
fn links_of(meta: &CachedMetadata) -> Vec<LinkValue> {
    meta.frontmatter_links
        .iter()
        .map(|l| LinkValue {
            path: l.link.clone(),
            display: Some(l.display_text.clone().unwrap_or_else(|| l.link.clone())),
        })
        .chain(meta.links.iter().map(link_value))
        .collect()
}

/// A cached link as a value. The path is the **written** target, not the
/// resolved one, and the display is Obsidian's own `displayText`, which is
/// set even with no alias — `[[Pumice]]` is the value `[[Pumice|Pumice]]`.
fn link_value(link: &LinkCache) -> LinkValue {
    LinkValue {
        path: link.link.clone(),
        display: Some(
            link.display_text
                .clone()
                .unwrap_or_else(|| link.link.clone()),
        ),
    }
}

/// The note's frontmatter as values, in file order.
fn properties_of(meta: &CachedMetadata, types: &PropertyTypes) -> IndexMap<String, Value> {
    let Some(serde_yaml::Value::Mapping(map)) = &meta.frontmatter else {
        return IndexMap::new();
    };
    let mut out = IndexMap::with_capacity(map.len());
    for (key, value) in map {
        let Some(key) = key.as_str() else { continue };
        out.insert(key.to_string(), property(key, value, types));
    }
    out
}

/// One frontmatter entry as a value.
///
/// The type comes from Silica's one property-typing rule — `types.json`, then
/// the reserved keys (`tags`, `aliases`), then inference from the value — and
/// only the *value* side is Bases' own. Two shapes a string can take are
/// Bases' alone and were observed: `stamp: 2026-01-02T03:04:05` is a DateTime
/// where the properties panel calls an ISO date-time `text`, and
/// `linkish: "[[Pumice]]"` is a Link where the panel calls it `text`.
fn property(key: &str, value: &serde_yaml::Value, types: &PropertyTypes) -> Value {
    match assigned_type(key, types) {
        Some(PropertyType::Tags) => Value::Tags(
            strings(value)
                .into_iter()
                .map(|t| match t.strip_prefix('#') {
                    Some(bare) => format!("#{bare}"),
                    None => format!("#{t}"),
                })
                .collect(),
        ),
        Some(PropertyType::MultiText | PropertyType::Aliases) => Value::List(match value {
            serde_yaml::Value::Sequence(items) => items.iter().map(scalar).collect(),
            serde_yaml::Value::Null => Vec::new(),
            one => vec![scalar(one)],
        }),
        Some(PropertyType::Number) => match value {
            serde_yaml::Value::Number(n) => Value::Number(n.as_f64().unwrap_or(f64::NAN)),
            other => scalar(other),
        },
        Some(PropertyType::Checkbox) => match value {
            serde_yaml::Value::Bool(b) => Value::Bool(*b),
            other => scalar(other),
        },
        // An assigned date type keeps the time flag the type asks for, even
        // where the written value carries the other one.
        Some(ty @ (PropertyType::Date | PropertyType::DateTime)) => {
            match value.as_str().and_then(parse_date) {
                Some(at) => Value::Date(DateValue {
                    has_time: ty == PropertyType::DateTime,
                    ..at
                }),
                None => scalar(value),
            }
        }
        Some(PropertyType::Text) | None => scalar(value),
    }
}

/// The type `types.json` or a reserved key fixes for `key`, if either does.
/// Inference is left to [`scalar`], which has the value in hand.
fn assigned_type(key: &str, types: &PropertyTypes) -> Option<PropertyType> {
    types
        .types
        .get(key)
        .and_then(|s| PropertyType::parse(s))
        .or_else(|| reserved_type(key))
}

/// A YAML value with no assigned type, typed the way Bases types one.
fn scalar(value: &serde_yaml::Value) -> Value {
    match value {
        serde_yaml::Value::Null => Value::Null,
        serde_yaml::Value::Bool(b) => Value::Bool(*b),
        serde_yaml::Value::Number(n) => Value::Number(n.as_f64().unwrap_or(f64::NAN)),
        serde_yaml::Value::String(s) => text(s),
        serde_yaml::Value::Sequence(items) => Value::List(items.iter().map(scalar).collect()),
        serde_yaml::Value::Mapping(map) => Value::Object(
            map.iter()
                .filter_map(|(k, v)| k.as_str().map(|k| (k.to_string(), scalar(v))))
                .collect(),
        ),
        serde_yaml::Value::Tagged(tagged) => scalar(&tagged.value),
    }
}

/// A string property. A whole `[[wikilink]]` is a Link and a date or
/// date-time is a Date; everything else is a String. The date parser is the
/// language's own, so `date("…")` and a property agree by construction.
fn text(s: &str) -> Value {
    if let Some(link) = wikilink(s) {
        return Value::Link(link);
    }
    match parse_date(s) {
        Some(at) => Value::Date(at),
        None => Value::Str(s.to_string()),
    }
}

/// A property whose whole value is one `[[target|alias]]`. Unlike a link in
/// the links list this carries no display unless the value wrote an alias:
/// there is no cached `displayText` behind a property value.
fn wikilink(s: &str) -> Option<LinkValue> {
    let inner = s.trim().strip_prefix("[[")?.strip_suffix("]]")?;
    if inner.contains("[[") || inner.contains("]]") {
        return None;
    }
    let parsed = silica_markdown::parse_linktext(inner);
    let path = inner.split_once('|').map_or(inner, |(t, _)| t).trim();
    if path.is_empty() {
        return None;
    }
    Some(LinkValue {
        path: path.to_string(),
        display: parsed.alias.map(str::to_string),
    })
}

/// A `tags` value as plain strings: a sequence item is taken whole, while a
/// lone scalar is split on commas and whitespace. The same shape
/// `silica_cache`'s own `frontmatter_tags` reads, which is what the tag pane
/// and `file.tags` already agree on.
fn strings(value: &serde_yaml::Value) -> Vec<String> {
    match value {
        serde_yaml::Value::Sequence(items) => items.iter().filter_map(scalar_string).collect(),
        other => scalar_string(other)
            .map(|s| {
                s.split(|c: char| c == ',' || c.is_whitespace())
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn scalar_string(value: &serde_yaml::Value) -> Option<String> {
    match value {
        serde_yaml::Value::String(s) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silica_bases::Context;
    use std::fs;

    /// A vault with one note of every property shape, one in a folder, one
    /// linking back, an attachment and the base itself.
    fn vault() -> (tempfile::TempDir, Arc<VaultState>) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join(".obsidian")).unwrap();
        fs::create_dir_all(root.join("Rocks/Deep")).unwrap();
        fs::write(
            root.join("Silica.md"),
            "---\n\
             status: todo\n\
             density: 1.21\n\
             flag: true\n\
             when: 2026-01-02\n\
             stamp: 2026-01-02T03:04:05\n\
             text: Frothed basalt\n\
             numish: \"42\"\n\
             empty: \"\"\n\
             nothing:\n\
             listish:\n  - alpha\n  - beta\n\
             nums:\n  - 3\n  - 1\n\
             linkish: \"[[Pumice]]\"\n\
             aliased: \"[[Pumice|Foam]]\"\n\
             shelf:\n  row: 1\n\
             tags:\n  - rock\n  - Igneous/extrusive\n\
             aliases:\n  - Cinder\n\
             ---\n\
             \n\
             Frothed basalt, see [[Pumice]] and [[Rocks/Deep/Basalt|deep]].\n\
             \n\
             ![[pic.png]]\n\
             \n\
             Some text with a #rock tag and #new.\n",
        )
        .unwrap();
        fs::write(root.join("Pumice.md"), "Floats. [[Silica]].\n").unwrap();
        fs::write(root.join("Rocks/Deep/Basalt.md"), "# Basalt\n").unwrap();
        fs::write(root.join("pic.png"), b"\x89PNG not really").unwrap();
        fs::write(root.join("Main.base"), "views:\n  - type: table\n").unwrap();
        let state = Arc::new(VaultState::load(&root.to_path_buf()).unwrap());
        (dir, state)
    }

    fn one(state: &Arc<VaultState>, path: &str) -> Arc<FileData> {
        file_data(state, path).unwrap_or_else(|| panic!("no file data for {path}"))
    }

    #[test]
    fn a_root_file_is_in_the_folder_slash() {
        let (_dir, state) = vault();
        let silica = one(&state, "Silica.md");
        assert_eq!(silica.folder, "/");
        // A note in a folder gets the folder's own vault-relative path.
        assert_eq!(one(&state, "Rocks/Deep/Basalt.md").folder, "Rocks/Deep");
    }

    #[test]
    fn a_name_drops_md_and_keeps_every_other_extension() {
        let (_dir, state) = vault();
        let note = one(&state, "Silica.md");
        assert_eq!(note.name, "Silica");
        assert_eq!(note.basename, "Silica");
        assert_eq!(note.fullname, "Silica.md");
        assert_eq!(note.path, "Silica.md");
        assert_eq!(note.ext, "md");

        let base = one(&state, "Main.base");
        assert_eq!(base.name, "Main.base");
        assert_eq!(base.basename, "Main");
        assert_eq!(base.fullname, "Main.base");
        assert_eq!(base.ext, "base");

        let deep = one(&state, "Rocks/Deep/Basalt.md");
        assert_eq!(deep.name, "Basalt");
        assert_eq!(deep.path, "Rocks/Deep/Basalt.md");
    }

    #[test]
    fn size_is_bytes_and_the_stamps_are_civil_date_times() {
        let (_dir, state) = vault();
        let note = one(&state, "Silica.md");
        let on_disk = fs::metadata(state.root.join("Silica.md")).unwrap().len();
        assert_eq!(note.size, on_disk as f64);
        // No disk read: the scan row is where all three of these come from.
        let (size, ctime, mtime_ms) = {
            let vault = state.vault();
            let row = vault
                .files()
                .iter()
                .find(|f| f.path == "Silica.md")
                .expect("scan row");
            (row.size, row.ctime, row.mtime)
        };
        assert_eq!(note.size, size as f64);
        let mtime = note.mtime.expect("mtime");
        assert!(mtime.has_time, "a file stamp is a DateTime, not a Date");
        assert_eq!(civil(mtime_ms).unwrap(), mtime);
        assert_eq!(civil(ctime).unwrap(), note.ctime.unwrap());
        assert!(mtime.at.format("%Y").to_string().parse::<i32>().unwrap() >= 2020);
    }

    #[test]
    fn tags_carry_their_hash_frontmatter_first_and_merge_case_insensitively() {
        let (_dir, state) = vault();
        let note = one(&state, "Silica.md");
        // `rock` is written in the properties and again inline; one value.
        assert_eq!(
            note.tags,
            vec!["#rock", "#Igneous/extrusive", "#new"],
            "frontmatter tags first, then the inline ones not already seen"
        );
    }

    #[test]
    fn links_are_the_written_targets_and_backlinks_are_the_linking_file() {
        let (_dir, state) = vault();
        let note = one(&state, "Silica.md");
        let links: Vec<(&str, Option<&str>)> = note
            .links
            .iter()
            .map(|l| (l.path.as_str(), l.display.as_deref()))
            .collect();
        assert_eq!(
            links,
            vec![
                // The property link comes first, as observed.
                ("Pumice", Some("Pumice")),
                ("Pumice", Some("Foam")),
                ("Pumice", Some("Pumice")),
                ("Rocks/Deep/Basalt", Some("deep")),
            ]
        );
        // An embed is its own list and is not among the links.
        let embeds: Vec<&str> = note.embeds.iter().map(|l| l.path.as_str()).collect();
        assert_eq!(embeds, vec!["pic.png"]);
        // `[[Pumice]]` renders as `[[Pumice|Pumice]]`, which is observed.
        assert_eq!(
            Value::Link(note.links[0].clone()).to_string(),
            "[[Pumice|Pumice]]"
        );

        // A backlink names the *linking file's* full path.
        let pumice = one(&state, "Pumice.md");
        let back: Vec<(&str, Option<&str>)> = pumice
            .backlinks
            .iter()
            .map(|l| (l.path.as_str(), l.display.as_deref()))
            .collect();
        assert_eq!(back, vec![("Silica.md", Some("Silica"))]);
        assert!(one(&state, "Rocks/Deep/Basalt.md")
            .backlinks
            .iter()
            .any(|l| l.path == "Silica.md"));
    }

    #[test]
    fn frontmatter_takes_obsidians_own_property_types() {
        let (_dir, state) = vault();
        let note = one(&state, "Silica.md");
        let p = |key: &str| note.properties.get(key).cloned().unwrap_or(Value::Null);
        assert_eq!(p("status"), Value::Str("todo".into()));
        assert_eq!(p("density"), Value::Number(1.21));
        assert_eq!(p("flag"), Value::Bool(true));
        assert_eq!(p("text"), Value::Str("Frothed basalt".into()));
        // A quoted number stays text: `numish + 1` is the String "421".
        assert_eq!(p("numish"), Value::Str("42".into()));
        assert_eq!(p("empty"), Value::Str(String::new()));
        assert_eq!(p("nothing"), Value::Null);
        assert_eq!(
            p("listish"),
            Value::List(vec![Value::str("alpha"), Value::str("beta")])
        );
        assert_eq!(
            p("nums"),
            Value::List(vec![Value::Number(3.0), Value::Number(1.0)])
        );
        assert_eq!(
            p("aliases"),
            Value::List(vec![Value::str("Cinder")]),
            "a reserved key is still a list of its own strings"
        );
        // Dates: `YYYY-MM-DD` is a Date, an ISO date-time a DateTime.
        let when = parse_date("2026-01-02").unwrap();
        assert_eq!(p("when"), Value::Date(when));
        assert!(!when.has_time);
        let stamp = parse_date("2026-01-02T03:04:05").unwrap();
        assert_eq!(p("stamp"), Value::Date(stamp));
        assert!(stamp.has_time);
        // Tags keep their `#`, which the file does not write.
        assert_eq!(
            p("tags"),
            Value::Tags(vec!["#rock".into(), "#Igneous/extrusive".into()])
        );
        // A whole wikilink is a Link; an alias is its display.
        assert_eq!(
            p("linkish"),
            Value::Link(LinkValue {
                path: "Pumice".into(),
                display: None
            })
        );
        assert_eq!(
            p("aliased"),
            Value::Link(LinkValue {
                path: "Pumice".into(),
                display: Some("Foam".into())
            })
        );
        // A nested mapping is an Object.
        assert!(matches!(p("shelf"), Value::Object(_)));
        // File order, as the note writes them.
        let keys: Vec<&str> = note.properties.keys().map(String::as_str).collect();
        assert_eq!(keys[..4], ["status", "density", "flag", "when"]);
    }

    #[test]
    fn types_json_wins_over_what_the_value_looks_like() {
        let (dir, _state) = vault();
        fs::write(
            dir.path().join(".obsidian/types.json"),
            r#"{"types":{"when":"datetime","numish":"number","density":"text","listish":"text"}}"#,
        )
        .unwrap();
        let state = Arc::new(VaultState::load(&dir.path().to_path_buf()).unwrap());
        let note = one(&state, "Silica.md");
        let p = |key: &str| note.properties.get(key).cloned().unwrap();
        // The written value is a bare date; the assigned type says date-time.
        match p("when") {
            Value::Date(d) => assert!(d.has_time),
            other => panic!("expected a date, got {other:?}"),
        }
        // A quoted "42" assigned `number` is still text, as Obsidian keeps
        // what was typed: the YAML value is a string.
        assert_eq!(p("numish"), Value::Str("42".into()));
        // A number assigned `text` keeps its number value.
        assert_eq!(p("density"), Value::Number(1.21));
        // A list assigned `text` is still the list the YAML holds.
        assert_eq!(
            p("listish"),
            Value::List(vec![Value::str("alpha"), Value::str("beta")])
        );
    }

    #[test]
    fn a_note_with_no_frontmatter_has_no_properties() {
        let (_dir, state) = vault();
        let note = one(&state, "Pumice.md");
        assert!(note.properties.is_empty());
        assert!(note.tags.is_empty());
    }

    #[test]
    fn env_holds_the_notes_and_the_base_file_is_this() {
        let (_dir, state) = vault();
        let env = env_for(&state, "Main.base");
        let paths: Vec<&str> = env.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["Pumice.md", "Rocks/Deep/Basalt.md", "Silica.md"]
        );
        assert!(
            !paths.contains(&"pic.png") && !paths.contains(&"Main.base"),
            "rows are notes: no attachments and not the base itself"
        );
        let base = env.base_file.clone().expect("this");
        assert_eq!(base.name, "Main.base");
        assert_eq!(base.folder, "/");
        assert!(base.properties.is_empty());
        assert!(env.now.format("%Y").to_string().parse::<i32>().unwrap() >= 2020);
    }

    #[test]
    fn a_base_the_scan_has_not_seen_still_answers_this() {
        let (_dir, state) = vault();
        let env = env_for(&state, "Later/Fresh.base");
        let base = env.base_file.expect("this");
        assert_eq!(base.name, "Fresh.base");
        assert_eq!(base.folder, "Later");
        assert_eq!(base.size, 0.0);
    }

    #[test]
    fn a_row_context_reads_the_values_a_statement_asks_for() {
        let (_dir, state) = vault();
        let env = env_for(&state, "Main.base");
        let file = env
            .files
            .iter()
            .find(|f| f.path == "Silica.md")
            .cloned()
            .unwrap();
        let ctx = silica_bases::SimpleContext {
            file: Some(file),
            base_file: env.base_file.clone(),
            files: env.files.clone(),
            ..Default::default()
        };
        // A bare ident is a frontmatter property, case-insensitively.
        assert_eq!(ctx.property("STATUS"), Some(Value::Str("todo".into())));
        assert_eq!(ctx.property("nosuchprop"), None);
        // `this` is the base file, never the row.
        assert_eq!(ctx.base_file().unwrap().name, "Main.base");
        assert_eq!(ctx.file().unwrap().name, "Silica");
        // A link target resolves to a row's file.
        assert_eq!(ctx.lookup_file("Pumice").unwrap().path, "Pumice.md");
    }

    #[test]
    fn an_unknown_path_has_no_file_data() {
        let (_dir, state) = vault();
        assert!(file_data(&state, "Nope.md").is_none());
    }

    /// What a rebuild costs on a vault the size `silica-bench` gates on.
    /// Ignored, because writing 20k notes is seconds of IO that every other
    /// test in the crate would pay for:
    /// `cargo test -p silica --release bases -- --ignored --nocapture`.
    #[test]
    #[ignore = "writes a 20k-note vault; run it by name"]
    fn twenty_thousand_notes_rebuild_in_well_under_the_index_gate() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join(".obsidian")).unwrap();
        for folder in 0..20 {
            fs::create_dir_all(root.join(format!("f{folder}"))).unwrap();
        }
        for note in 0..20_000u32 {
            let folder = note % 20;
            let link = (note + 1) % 20_000;
            fs::write(
                root.join(format!("f{folder}/Note {note}.md")),
                format!(
                    "---\nstatus: todo\ndensity: 1.21\nwhen: 2026-01-02\n\
                     tags:\n  - rock\naliases:\n  - A{note}\n---\n\n\
                     Body of {note}, see [[Note {link}]].\n\n#inline\n"
                ),
            )
            .unwrap();
        }
        fs::write(root.join("Main.base"), "views:\n  - type: table\n").unwrap();
        let loaded = std::time::Instant::now();
        let state = Arc::new(VaultState::load(&root.to_path_buf()).unwrap());
        let load_ms = loaded.elapsed().as_secs_f64() * 1000.;
        let built = std::time::Instant::now();
        let env = env_for(&state, "Main.base");
        let build_ms = built.elapsed().as_secs_f64() * 1000.;
        println!(
            "load {load_ms:.0} ms, env_for {build_ms:.0} ms for {} rows",
            env.files.len()
        );
        assert_eq!(env.files.len(), 20_000);
        assert_eq!(env.files[0].tags, vec!["#rock", "#inline"]);
    }
}
