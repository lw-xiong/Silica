//! The rows Obsidian 1.14.4 wrote, asserted against what this crate writes.
//!
//! Every case here was observed with a throwaway plugin driving Obsidian's
//! own suggesters over the vault built below — the trigger it fired, the
//! item it offered and the text each accept key inserted. See
//! [[Specification#Observed autocompletion behavior, Obsidian 1.14.4]] and
//! [[Development#Autocompletion, 2026-10-18]]. A row that stops matching is
//! a regression against Obsidian, not against this test.

use silica_cache::links::LinkFormat;
use silica_cache::{index_markdown, LinkResolver, MetadataCache};
use silica_formats::FileKind;
use silica_suggest::{insert, suggestions, trigger, Accept, Item, Options, Vault};
use silica_vault::VaultFile;

/// The probe vault: a duplicate basename, a file with a space, aliases,
/// headings, tags and an attachment.
const NOTES: &[(&str, &str)] = &[
    (
        "Alpha.md",
        "---\naliases:\n  - Alpha Alias\n  - Second Name\ntags:\n  - project/alpha\n---\n\n\
         # Alpha\n\nSome text with #inline-tag and #project/alpha here.\n\n\
         ## First Heading\n\nA paragraph. ^blockone\n\n### Nested Heading\n\nAnother paragraph. ^block-two\n",
    ),
    ("Beta Note.md", "# Beta Note\n\nBody with #beta-tag.\n"),
    (
        "Sub/Alpha.md",
        "---\naliases:\n  - Sub Alias\n---\n\n# Sub Alpha\n\nDuplicate basename on purpose.\n",
    ),
    ("Sub/Gamma.md", "# Gamma\n\n## Gamma Heading\n"),
    ("Sub/Source.md", "# Source\n"),
    ("Scratch.md", "typing happens here\n"),
];

const ATTACHMENTS: &[&str] = &["pic.png"];

/// A note's text, which is what a block row's label reads.
fn note_text(path: &str) -> Option<String> {
    NOTES
        .iter()
        .find(|(p, _)| *p == path)
        .map(|(_, src)| (*src).to_string())
}

/// Modification times in the order the probe's files were written, so the
/// empty-query order is the one Obsidian showed.
fn files() -> Vec<VaultFile> {
    let mut out = Vec::new();
    for (ix, (path, _)) in NOTES.iter().enumerate() {
        out.push(VaultFile {
            path: (*path).to_string(),
            kind: FileKind::from_path(path),
            size: 0,
            mtime: 1_000 + ix as u64,
            ctime: 1_000 + ix as u64,
        });
    }
    for path in ATTACHMENTS {
        out.push(VaultFile {
            path: (*path).to_string(),
            kind: FileKind::from_path(path),
            size: 0,
            mtime: 1_000,
            ctime: 1_000,
        });
    }
    out
}

fn cache() -> MetadataCache {
    let mut resolver = LinkResolver::default();
    for (path, _) in NOTES {
        resolver.insert(path);
    }
    for path in ATTACHMENTS {
        resolver.insert(path);
    }
    MetadataCache::new(
        resolver,
        NOTES.iter().map(|(path, src)| (*path, index_markdown(src))),
    )
}

struct Fixture {
    files: Vec<VaultFile>,
    cache: MetadataCache,
}

impl Fixture {
    fn new() -> Fixture {
        Fixture {
            files: files(),
            cache: cache(),
        }
    }

    fn vault<'a>(&'a self, source: &'a str) -> Vault<'a> {
        Vault {
            files: &self.files,
            cache: &self.cache,
            source,
            show_unsupported: false,
            text: &note_text,
        }
    }

    /// Types `line` in `source`, picks the first offered row whose label
    /// contains `want`, presses `accept`, and returns the line that leaves.
    fn accept(
        &self,
        source: &str,
        line: &str,
        want: &str,
        accept: Accept,
        opts: &Options,
    ) -> String {
        let (line, col) = match line.find('|') {
            // A `|` in the case marks the cursor when it is not at the end.
            Some(ix) if line.ends_with("]]") => (line.replace('|', ""), ix),
            _ => (line.to_string(), line.len()),
        };
        let v = self.vault(source);
        let t = trigger(&line, col).expect("the line triggers a suggester");
        let items = suggestions(&t, &v);
        let item = items
            .iter()
            .find(|s| s.label.contains(want))
            .unwrap_or_else(|| {
                panic!(
                    "no row matching {want:?} for {line:?}; offered {:?}",
                    items.iter().map(|s| &s.label).collect::<Vec<_>>()
                )
            });
        let ins = insert(&t, &item.item, accept, &v, opts);
        let mut out = line.clone();
        out.replace_range(t.range.start..t.range.end + ins.consume, &ins.text);
        out
    }

    /// The same, pressing Enter on the first row offered.
    fn first(&self, source: &str, line: &str, opts: &Options) -> String {
        self.accept(source, line, "", Accept::Whole, opts)
    }
}

fn wiki(format: LinkFormat) -> Options {
    Options {
        format,
        markdown_links: false,
    }
}

fn md(format: LinkFormat) -> Options {
    Options {
        format,
        markdown_links: true,
    }
}

#[test]
fn wikilinks_in_the_shortest_format() {
    let f = Fixture::new();
    let o = wiki(LinkFormat::Shortest);
    let at = |line, want| f.accept("Scratch.md", line, want, Accept::Whole, &o);
    assert_eq!(at("[[beta no", "Beta"), "[[Beta Note]]");
    assert_eq!(at("[[ga", "Gamma"), "[[Gamma]]");
    // A path with a folder takes the basename as display text.
    assert_eq!(at("[[al", "Sub/Alpha"), "[[Sub/Alpha|Alpha]]");
    assert_eq!(at("[[al", "Alpha"), "[[Alpha]]");
    assert_eq!(at("[[alpha ali", "Alpha Alias"), "[[Alpha|Alpha Alias]]");
    assert_eq!(at("[[sub alias", "Sub Alias"), "[[Sub/Alpha|Sub Alias]]");
    assert_eq!(at("[[Alpha|", "Alpha Alias"), "[[Alpha|Alpha Alias]]");
    assert_eq!(at("[[Alpha|disp", "disp"), "[[Alpha|disp]]");
    assert_eq!(
        at("[[Alpha#first", "First Heading"),
        "[[Alpha#First Heading]]"
    );
    // An attachment never takes display text.
    assert_eq!(at("[[pi", "pic.png"), "[[pic.png]]");
    assert_eq!(at("![[beta no", "Beta"), "![[Beta Note]]");
    assert_eq!(at("![[pi", "pic.png"), "![[pic.png]]");
    // Nothing matched: the query is linked as typed, and no note is created.
    assert_eq!(
        at("[[No Such Note At All", "No Such"),
        "[[No Such Note At All]]"
    );
}

#[test]
fn a_heading_keeps_the_path_as_typed() {
    let f = Fixture::new();
    for format in [
        LinkFormat::Shortest,
        LinkFormat::Relative,
        LinkFormat::Absolute,
    ] {
        let o = wiki(format);
        // `shortest` would write `Gamma`, and `absolute` `Sub/Gamma`; a
        // heading pick writes back exactly what was typed, with no display
        // text however deep the folder.
        assert_eq!(
            f.accept(
                "Scratch.md",
                "[[Gamma#gamma h",
                "Gamma Heading",
                Accept::Whole,
                &o
            ),
            "[[Gamma#Gamma Heading]]"
        );
        assert_eq!(
            f.accept(
                "Scratch.md",
                "[[Sub/Gamma#",
                "Gamma Heading",
                Accept::Whole,
                &o
            ),
            "[[Sub/Gamma#Gamma Heading]]"
        );
    }
}

#[test]
fn wikilinks_in_the_other_formats() {
    let f = Fixture::new();
    let rel = wiki(LinkFormat::Relative);
    let abs = wiki(LinkFormat::Absolute);
    let at = |o: &Options, source, line, want| f.accept(source, line, want, Accept::Whole, o);

    // From the root, `relative` and `absolute` both name the folder.
    assert_eq!(
        at(&rel, "Scratch.md", "[[ga", "Gamma"),
        "[[Sub/Gamma|Gamma]]"
    );
    assert_eq!(
        at(&abs, "Scratch.md", "[[ga", "Gamma"),
        "[[Sub/Gamma|Gamma]]"
    );

    // From inside `Sub`, only `relative` climbs out.
    assert_eq!(
        at(&rel, "Sub/Source.md", "[[beta no", "Beta"),
        "[[../Beta Note|Beta Note]]"
    );
    assert_eq!(at(&rel, "Sub/Source.md", "[[ga", "Gamma"), "[[Gamma]]");
    assert_eq!(at(&rel, "Sub/Source.md", "[[al", "Sub/Alpha"), "[[Alpha]]");
    assert_eq!(
        at(&rel, "Sub/Source.md", "[[al", "Alpha"),
        "[[../Alpha|Alpha]]"
    );
    // An attachment keeps its extension and still takes no display text.
    assert_eq!(
        at(&rel, "Sub/Source.md", "[[pi", "pic.png"),
        "[[../pic.png]]"
    );
    assert_eq!(
        at(&abs, "Sub/Source.md", "[[beta no", "Beta"),
        "[[Beta Note]]"
    );
    assert_eq!(
        at(&abs, "Sub/Source.md", "[[ga", "Gamma"),
        "[[Sub/Gamma|Gamma]]"
    );
}

#[test]
fn markdown_links() {
    let f = Fixture::new();
    let o = md(LinkFormat::Shortest);
    let at = |line, want| f.accept("Scratch.md", line, want, Accept::Whole, &o);
    // The text is the basename, the destination keeps `.md`, and only
    // spaces are encoded.
    assert_eq!(at("[[beta no", "Beta"), "[Beta Note](Beta%20Note.md)");
    assert_eq!(at("[[ga", "Gamma"), "[Gamma](Gamma.md)");
    assert_eq!(at("[[al", "Sub/Alpha"), "[Alpha](Sub/Alpha.md)");
    assert_eq!(at("[[alpha ali", "Alpha Alias"), "[Alpha Alias](Alpha.md)");
    assert_eq!(
        at("[[Alpha#first", "First Heading"),
        "[First Heading](Alpha.md#First%20Heading)"
    );
    // An attachment's text drops the extension; the destination keeps it.
    assert_eq!(at("[[pi", "pic.png"), "[pic](pic.png)");
    assert_eq!(at("![[beta no", "Beta"), "![Beta Note](Beta%20Note.md)");
    assert_eq!(at("![[pi", "pic.png"), "![pic](pic.png)");
    // With no file behind it: no `.md`, and no text.
    assert_eq!(
        at("[[No Such Note At All", "No Such"),
        "[](No%20Such%20Note%20At%20All)"
    );
    assert_eq!(at("[[Alpha|disp", "disp"), "[disp](Alpha)");
    assert_eq!(
        f.accept(
            "Scratch.md",
            "[[Sub/Gamma#",
            "Gamma Heading",
            Accept::Whole,
            &o
        ),
        "[Gamma Heading](Sub/Gamma.md#Gamma%20Heading)"
    );
}

#[test]
fn the_three_accept_keys_differ() {
    let f = Fixture::new();
    let o = wiki(LinkFormat::Shortest);
    let case = |accept| f.accept("Scratch.md", "[[beta no", "Beta", accept, &o);
    assert_eq!(case(Accept::Whole), "[[Beta Note]]");
    assert_eq!(case(Accept::Inside), "[[Beta Note]]");
    // Shift+Enter ignores the row and links the query as typed.
    assert_eq!(case(Accept::Typed), "[[beta no]]");
    assert_eq!(
        f.accept(
            "Scratch.md",
            "[[alpha ali",
            "Alpha Alias",
            Accept::Typed,
            &o
        ),
        "[[alpha ali]]"
    );
    assert_eq!(
        f.accept(
            "Scratch.md",
            "[[be",
            "Beta",
            Accept::Typed,
            &md(LinkFormat::Shortest)
        ),
        "[](be)"
    );
}

#[test]
fn the_cursor_lands_where_obsidian_leaves_it() {
    let f = Fixture::new();
    let v = f.vault("Scratch.md");
    let line = "[[beta no";
    let t = trigger(line, line.len()).expect("triggers");
    let items = suggestions(&t, &v);
    let item = &items
        .iter()
        .find(|s| s.label.contains("Beta"))
        .expect("Beta Note is offered")
        .item;
    let o = wiki(LinkFormat::Shortest);
    // Enter sits after the brackets; Tab inside them, so a `#heading` can
    // follow what was just accepted.
    assert_eq!(
        insert(&t, item, Accept::Whole, &v, &o).cursor,
        "[[Beta Note]]".len()
    );
    assert_eq!(
        insert(&t, item, Accept::Inside, &v, &o).cursor,
        "[[Beta Note".len()
    );
    // A markdown link has nowhere to stand, so Tab lands where Enter does.
    let o = md(LinkFormat::Shortest);
    let whole = insert(&t, item, Accept::Whole, &v, &o);
    assert_eq!(whole.cursor, whole.text.len());
    assert_eq!(
        insert(&t, item, Accept::Inside, &v, &o).cursor,
        whole.text.len()
    );
    // With no text to show the cursor goes between the brackets.
    let typed = insert(&t, &Item::Literal, Accept::Whole, &v, &o);
    assert_eq!((typed.text.as_str(), typed.cursor), ("[](beta%20no)", 1));
}

#[test]
fn an_existing_closing_pair_is_consumed() {
    let f = Fixture::new();
    let o = wiki(LinkFormat::Shortest);
    // The `|` marks the cursor: `[[be|]]`.
    assert_eq!(
        f.accept("Scratch.md", "[[be|]]", "Beta", Accept::Whole, &o),
        "[[Beta Note]]"
    );
    let line = "[[be]]";
    let t = trigger(line, 4).expect("triggers inside the brackets");
    assert_eq!(t.after, "]]");
    let v = f.vault("Scratch.md");
    assert_eq!(insert(&t, &Item::Literal, Accept::Whole, &v, &o).consume, 2);
}

#[test]
fn tags_are_written_with_a_space_on_enter_only() {
    let f = Fixture::new();
    let o = wiki(LinkFormat::Shortest);
    assert_eq!(
        f.accept("Scratch.md", "#inl", "inline-tag", Accept::Whole, &o),
        "#inline-tag "
    );
    assert_eq!(
        f.accept("Scratch.md", "#inl", "inline-tag", Accept::Inside, &o),
        "#inline-tag"
    );
    // The bare `#` offers every tag alphabetically, parents included.
    assert_eq!(f.first("Scratch.md", "#", &o), "#beta-tag ");
    let v = f.vault("Scratch.md");
    let t = trigger("#", 1).expect("triggers");
    let labels: Vec<String> = suggestions(&t, &v)
        .iter()
        .map(|s| s.label.clone())
        .collect();
    assert_eq!(
        labels,
        ["beta-tag", "inline-tag", "project", "project/alpha"]
    );
}

#[test]
fn an_empty_query_is_ordered_by_modification_time() {
    let f = Fixture::new();
    let v = f.vault("Scratch.md");
    let t = trigger("[[", 2).expect("triggers");
    let items = suggestions(&t, &v);
    // Scratch.md is the newest of the fixture, and the aliases of a file
    // follow it.
    assert_eq!(items[0].label, "Scratch");
    assert_eq!(items[1].label, "Sub/Source");
    let labels: Vec<&str> = items.iter().map(|s| s.label.as_str()).collect();
    assert!(
        labels.contains(&"Alpha Alias"),
        "aliases are offered: {labels:?}"
    );
    assert!(
        labels.contains(&"pic.png"),
        "attachments are offered: {labels:?}"
    );
    // Unlike the quick switcher, no unresolved link target is offered.
    assert!(!labels.contains(&"No Such Note At All"));
    assert_eq!(items.len(), 6 + 1 + 3);
}

#[test]
fn headings_come_in_document_order_with_the_h1() {
    let f = Fixture::new();
    let v = f.vault("Scratch.md");
    let t = trigger("[[Alpha#", 8).expect("triggers");
    let labels: Vec<String> = suggestions(&t, &v)
        .iter()
        .map(|s| s.label.clone())
        .collect();
    assert_eq!(labels, ["Alpha", "First Heading", "Nested Heading"]);
    // `[[#` is the note being edited.
    let v = f.vault("Alpha.md");
    let t = trigger("[[#", 3).expect("triggers");
    let labels: Vec<String> = suggestions(&t, &v)
        .iter()
        .map(|s| s.label.clone())
        .collect();
    assert_eq!(labels, ["Alpha", "First Heading", "Nested Heading"]);
}

#[test]
fn every_block_is_offered_in_document_order() {
    let f = Fixture::new();
    let v = f.vault("Scratch.md");
    let t = trigger("[[Alpha#^", 9).expect("triggers");
    let rows = suggestions(&t, &v);
    // The rows Obsidian offered for this note, in this order.
    let labels: Vec<String> = rows.iter().map(|s| s.label.clone()).collect();
    assert_eq!(
        labels,
        [
            "# Alpha",
            "Some text with #inline-tag and #project/alpha here.",
            "## First Heading",
            "A paragraph.",
            "### Nested Heading",
            "Another paragraph.",
        ]
    );
    // An id a block already carries is stripped from its label and shown
    // beside it; a block without one shows nothing there.
    let detail: Vec<String> = rows.iter().filter_map(|s| s.detail.clone()).collect();
    assert_eq!(detail, ["^blockone", "^block-two"]);
}

#[test]
fn a_block_query_matches_the_text_or_the_id() {
    let f = Fixture::new();
    let v = f.vault("Scratch.md");
    let rows = |line: &str| {
        let t = trigger(line, line.len()).expect("triggers");
        suggestions(&t, &v)
            .iter()
            .map(|s| s.label.clone())
            .collect::<Vec<_>>()
    };
    // Substring of the text, anywhere in it.
    assert_eq!(rows("[[Alpha#^ara"), ["A paragraph.", "Another paragraph."]);
    // The id itself, whatever its case.
    assert_eq!(rows("[[Alpha#^blockone"), ["A paragraph."]);
    assert_eq!(rows("[[Alpha#^BLOCKONE"), ["A paragraph."]);
    assert!(rows("[[Alpha#^zzz").is_empty());
}

#[test]
fn a_block_keeps_the_path_as_typed_and_shows_no_text() {
    let f = Fixture::new();
    let o = wiki(LinkFormat::Shortest);
    // `Gamma` is ambiguous in the fixture, and a block pick still writes
    // back exactly what was typed, like a heading pick.
    assert_eq!(
        f.accept(
            "Scratch.md",
            "[[Alpha#^block",
            "A paragraph.",
            Accept::Whole,
            &o
        ),
        "[[Alpha#^blockone]]"
    );
    assert_eq!(
        f.accept(
            "Scratch.md",
            "[[Alpha#^block-two",
            "Another",
            Accept::Whole,
            &o
        ),
        "[[Alpha#^block-two]]"
    );
    // Markdown style: `.md` kept, no display text, cursor in the brackets.
    let md = md(LinkFormat::Shortest);
    assert_eq!(
        f.accept(
            "Scratch.md",
            "[[Alpha#^block",
            "A paragraph.",
            Accept::Whole,
            &md
        ),
        "[](Alpha.md#^blockone)"
    );
}

#[test]
fn blocks_of_the_note_being_edited_come_from_the_empty_path() {
    let f = Fixture::new();
    let v = f.vault("Alpha.md");
    let t = trigger("[[#^", 4).expect("triggers");
    let labels: Vec<String> = suggestions(&t, &v)
        .iter()
        .map(|s| s.label.clone())
        .collect();
    assert_eq!(labels.first().map(String::as_str), Some("# Alpha"));
    assert_eq!(labels.len(), 6);
}
