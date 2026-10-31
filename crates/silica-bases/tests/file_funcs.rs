//! File and Link against Obsidian 1.14.5's own answers.
//!
//! Every case marked *observed* is lifted from the CDP run of 2026-10-25
//! (`tools/obsidian-probe/bases-expr.mjs`, raw results in
//! `tools/obsidian-probe/results/2026-10-25-bases-expr.json`) — steps
//! `e06-values`, `e08-methods`, `e14-function-table` and
//! `e17-case-and-namespaces`. The row is that run's own row: the note
//! `Silica.md` at the root of the probe vault, whose frontmatter and body are
//! the fixture in the probe script.
//!
//! Two of the run's rendered strings are the *cell's* text rather than
//! `toString()`, and the cell is not this crate's business: a List cell joins
//! with `,` where `[3, 1, 2].toString()` is `3, 1, 2` (observed in the same
//! run), and a Link cell shows the link's text where `linkish.toString()` is
//! `[[Pumice]]` (also observed). So `file.tags`, `file.links` and
//! `file.backlinks` are checked here against the `, ` join that
//! `List.toString()` is observed to use, and `file.asLink()` against both its
//! `toString()` and the text the cell showed.

use silica_bases::{eval_str, DateValue, FileData, LinkValue, SimpleContext, Value};

/// The run's row: `Silica.md`, at the vault root.
///
/// `file.name` is `Silica` and `file.path` is `Silica.md`, so `name` is the
/// display name with the `.md` dropped; `basename` was observed as `Silica`
/// too, and `fullname` is the file name with its extension — see the module
/// header of `funcs::file` for the evidence and for what is `unprobed`.
fn silica() -> FileData {
    let mut file = FileData {
        name: "Silica".into(),
        basename: "Silica".into(),
        fullname: "Silica.md".into(),
        path: "Silica.md".into(),
        folder: "/".into(),
        ext: "md".into(),
        size: 483.0,
        ctime: Some(stamp("2026-10-25T15:28:44")),
        mtime: Some(stamp("2026-10-25T15:28:44")),
        // Merged from the `tags` property and the inline `#rock`, with the
        // `#` Obsidian carries.
        tags: vec!["#rock".into(), "#igneous/extrusive".into()],
        // The frontmatter `linkish: "[[Pumice]]"` and the body's `[[Pumice]]`
        // and `[[Basalt]]`: three, in the order the run rendered them.
        links: vec![
            link("Pumice", "Pumice"),
            link("Pumice", "Pumice"),
            link("Basalt", "Basalt"),
        ],
        embeds: vec![],
        backlinks: vec![link("Pumice.md", "Pumice")],
        ..FileData::default()
    };
    // The fixture note's frontmatter, in file order.
    let props = &mut file.properties;
    props.insert("type".into(), Value::str("rock"));
    props.insert("status".into(), Value::str("todo"));
    props.insert("density".into(), Value::Number(1.21));
    props.insert("silica".into(), Value::Number(50.0));
    props.insert("count".into(), Value::Number(3.0));
    props.insert("flag".into(), Value::Bool(true));
    props.insert("off".into(), Value::Bool(false));
    props.insert("when".into(), Value::Date(day("2026-01-02T00:00:00")));
    props.insert("stamp".into(), Value::Date(stamp("2026-01-02T03:04:05")));
    props.insert("text".into(), Value::str("Frothed basalt"));
    props.insert("empty".into(), Value::str(""));
    props.insert(
        "listish".into(),
        Value::list(vec![
            Value::str("alpha"),
            Value::str("beta"),
            Value::str("alpha"),
        ]),
    );
    props.insert(
        "linkish".into(),
        Value::Link(LinkValue {
            path: "Pumice".into(),
            display: None,
        }),
    );
    props.insert(
        "tags".into(),
        Value::list(vec![Value::str("rock"), Value::str("igneous/extrusive")]),
    );
    props.insert("aliases".into(), Value::list(vec![Value::str("Cinder")]));
    file
}

/// A note of the probe vault that the row links to, as the host would resolve
/// it. `SimpleContext::lookup_file` matches a path, a name or a basename, so
/// the written target `Pumice` finds `Pumice.md`.
fn note(name: &str) -> FileData {
    FileData {
        name: name.into(),
        basename: name.into(),
        fullname: format!("{name}.md"),
        path: format!("{name}.md"),
        folder: "/".into(),
        ext: "md".into(),
        ..FileData::default()
    }
}

fn link(path: &str, display: &str) -> LinkValue {
    LinkValue {
        path: path.into(),
        display: Some(display.into()),
    }
}

fn stamp(s: &str) -> DateValue {
    DateValue {
        at: s.parse().expect("a date-time the test wrote itself"),
        has_time: true,
    }
}

fn day(s: &str) -> DateValue {
    DateValue {
        has_time: false,
        ..stamp(s)
    }
}

/// The probe vault: the row, the two notes it links to, and the base file
/// `Main.base`, which is what `this` is.
fn vault() -> SimpleContext {
    let base = FileData {
        name: "Main.base".into(),
        basename: "Main".into(),
        fullname: "Main.base".into(),
        path: "Main.base".into(),
        folder: "/".into(),
        ext: "base".into(),
        ..FileData::default()
    };
    let mut ctx = SimpleContext::new()
        .with_file(silica())
        .with_vault_file(note("Pumice"))
        .with_vault_file(note("Basalt"))
        .with_vault_file(note("Bare"));
    ctx.base_file = Some(std::sync::Arc::new(base));
    ctx
}

/// What a cell shows: the value's `toString()`, or Obsidian's refusal, which
/// the view renders as the cell's content just the same.
fn render(expr: &str, ctx: &SimpleContext) -> String {
    match eval_str(expr, ctx) {
        Ok(v) => v.to_string(),
        Err(e) => e.message,
    }
}

fn check(cases: &[(&str, &str)]) {
    let ctx = vault();
    let mut wrong = Vec::new();
    for (expr, want) in cases {
        let got = render(expr, &ctx);
        if got != *want {
            wrong.push(format!("{expr}\n  want {want:?}\n  got  {got:?}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

// --- properties -------------------------------------------------------------

#[test]
fn file_properties_are_the_recorded_ones() {
    check(&[
        // Observed, e17 and e14.
        ("file", "Silica.md"),
        ("file.toString()", "Silica.md"),
        ("file.isTruthy()", "true"),
        ("file.name", "Silica"),
        ("file.basename", "Silica"),
        ("file.path", "Silica.md"),
        ("file.folder", "/"),
        ("file.ext", "md"),
        ("file.size", "483"),
        ("file.ctime", "2026-10-25T15:28:44"),
        ("file.mtime", "2026-10-25T15:28:44"),
        ("file.file", "Silica.md"),
        ("file.properties.status", "todo"),
        // `fullname` is in the completion list but was never evaluated:
        // `unprobed`, and the reading is the file name with its extension.
        ("file.fullname", "Silica.md"),
        // Every name is case-insensitive; `file.NAME` and `file.Name` are
        // observed, the rest follow the same dispatch.
        ("file.NAME", "Silica"),
        ("file.Name", "Silica"),
        ("file.PATH", "Silica.md"),
        ("file.properties.STATUS", "todo"),
        // The list-ish properties, through `List.toString()`'s observed
        // `", "` join rather than the cell's own `,`.
        ("file.tags", "#rock, #igneous/extrusive"),
        (
            "file.links",
            "[[Pumice|Pumice]], [[Pumice|Pumice]], [[Basalt|Basalt]]",
        ),
        ("file.embeds", ""),
        ("file.backlinks", "[[Pumice.md|Pumice]]"),
    ]);
}

#[test]
fn an_unknown_property_is_obsidians_own_refusal() {
    // Observed, e08: every name off the table is refused by this wording, and
    // a function name reached without its parentheses is one of those.
    check(&[
        (
            "file.constructor",
            "Cannot find \"constructor\" on type File",
        ),
        ("file.toString", "Cannot find \"toString\" on type File"),
        ("file.getLinks", "Cannot find \"getLinks\" on type File"),
        ("file.nosuch", "Cannot find \"nosuch\" on type File"),
    ]);
}

#[test]
fn an_unknown_function_is_obsidians_own_refusal() {
    // Observed, e08.
    check(&[
        (
            "file.equals()",
            "Cannot find function \"equals\" on type File",
        ),
        (
            "file.looseEquals()",
            "Cannot find function \"looseEquals\" on type File",
        ),
        (
            "file.getLinks()",
            "Cannot find function \"getLinks\" on type File",
        ),
        (
            "file.getTags()",
            "Cannot find function \"getTags\" on type File",
        ),
        ("file.keys()", "Cannot find function \"keys\" on type File"),
        (
            "file.renderTo()",
            "Cannot find function \"renderTo\" on type File",
        ),
        (
            "file.lower()",
            "Cannot find function \"lower\" on type File",
        ),
    ]);
}

// --- hasTag -----------------------------------------------------------------

#[test]
fn hastag_is_the_one_case_insensitive_value_comparison() {
    check(&[
        // Observed, e17: all three are true on a note tagged `#rock`, and the
        // `#` is optional.
        ("file.hasTag(\"rock\")", "true"),
        ("file.hasTag(\"Rock\")", "true"),
        ("file.hasTag(\"#rock\")", "true"),
        // Observed, e14: no values at all is false, not a refusal.
        ("file.hasTag()", "false"),
        ("file.hasTag(\"nope\")", "false"),
        // "any of the values": variadic.
        ("file.hasTag(\"nope\", \"rock\")", "true"),
        ("file.hasTag(\"nope\", \"nope2\")", "false"),
        // The nested tag, whole and by case.
        ("file.hasTag(\"igneous/extrusive\")", "true"),
        ("file.hasTag(\"#IGNEOUS/Extrusive\")", "true"),
        // `unprobed`: a parent of a nested tag matches, as it does in
        // Obsidian's search and in `silica-search`.
        ("file.hasTag(\"igneous\")", "true"),
        ("file.hasTag(\"extrusive\")", "false"),
        ("file.hasTag(\"roc\")", "false"),
        // `unprobed`: the signature says String, so a RegExp is the same
        // refusal every other String parameter gives.
        (
            "file.hasTag(/zz/)",
            "Type error in \"hasTag\", parameter \"values\". Expected String, given RegExp.",
        ),
    ]);
}

// --- hasProperty, inFolder, hasLink ----------------------------------------

#[test]
fn hasproperty_asks_the_notes_own_key_set() {
    check(&[
        ("file.hasProperty(\"status\")", "true"),
        // Names are case-insensitive.
        ("file.hasProperty(\"STATUS\")", "true"),
        ("file.hasProperty(\"linkish\")", "true"),
        // Present and empty is still present.
        ("file.hasProperty(\"empty\")", "true"),
        ("file.hasProperty(\"nosuch\")", "false"),
        // Values are not: a property is matched by name, never by value.
        ("file.hasProperty(\"todo\")", "false"),
    ]);
}

#[test]
fn infolder_matches_the_root_and_the_folders_above() {
    check(&[
        // The row is at the root, where `file.folder` is `/`.
        ("file.inFolder(\"/\")", "true"),
        ("file.inFolder(\"\")", "true"),
        ("file.inFolder(\"Notes\")", "false"),
    ]);

    // A row in a folder. `unprobed`: a folder holds the files of its
    // subfolders, and the comparison ignores case, as vault paths do.
    let nested = FileData {
        name: "Tuff".into(),
        basename: "Tuff".into(),
        fullname: "Tuff.md".into(),
        path: "Rocks/Volcanic/Tuff.md".into(),
        folder: "Rocks/Volcanic".into(),
        ext: "md".into(),
        ..FileData::default()
    };
    let ctx = SimpleContext::new().with_file(nested);
    for (expr, want) in [
        ("file.inFolder(\"Rocks/Volcanic\")", "true"),
        ("file.inFolder(\"Rocks\")", "true"),
        ("file.inFolder(\"rocks\")", "true"),
        ("file.inFolder(\"/Rocks/\")", "true"),
        ("file.inFolder(\"/\")", "true"),
        ("file.inFolder(\"Roc\")", "false"),
        ("file.inFolder(\"Volcanic\")", "false"),
    ] {
        assert_eq!(render(expr, &ctx), want, "{expr}");
    }
}

#[test]
fn haslink_resolves_both_sides_through_the_host() {
    check(&[
        // The written target is `Pumice`; the file's path is `Pumice.md`.
        ("file.hasLink(\"Pumice\")", "true"),
        ("file.hasLink(\"Pumice.md\")", "true"),
        ("file.hasLink(\"Basalt\")", "true"),
        // A Link and a File are both `dest`.
        ("file.hasLink(linkish)", "true"),
        ("file.hasLink(linkish.asFile())", "true"),
        // The row does not link to itself, and `Bare.md` is unlinked.
        ("file.hasLink(file)", "false"),
        ("file.hasLink(\"Bare\")", "false"),
    ]);

    // A target the host cannot resolve still compares by what was written.
    let mut ghosted = silica();
    ghosted.links = vec![LinkValue {
        path: "Ghost".into(),
        display: None,
    }];
    let ctx = SimpleContext::new().with_file(ghosted);
    assert_eq!(render("file.hasLink(\"Ghost\")", &ctx), "true");
    assert_eq!(render("file.hasLink(\"ghost\")", &ctx), "true");
    assert_eq!(render("file.hasLink(\"Pumice\")", &ctx), "false");
}

// --- asLink -----------------------------------------------------------------

#[test]
fn aslink_is_a_link_to_the_files_own_path() {
    let ctx = vault();
    let value = eval_str("file.asLink()", &ctx).expect("a File has asLink()");
    assert_eq!(
        value,
        Value::Link(LinkValue {
            path: "Silica.md".into(),
            display: None,
        })
    );
    // Observed: the cell renders `Silica.md`, which is a Link's text when it
    // carries no alias. `toString()` is the wikilink.
    let Value::Link(l) = &value else {
        unreachable!("just asserted a Link")
    };
    assert_eq!(l.text(), "Silica.md");
    assert_eq!(value.to_string(), "[[Silica.md]]");

    // `unprobed`: the optional `display` becomes the alias, and a null one is
    // no alias at all.
    check(&[
        ("file.asLink(\"Cinder\")", "[[Silica.md|Cinder]]"),
        ("file.asLink(12)", "[[Silica.md|12]]"),
        ("file.asLink(nosuchprop)", "[[Silica.md]]"),
    ]);
}

// --- the Link's own two names ----------------------------------------------

#[test]
fn a_link_resolves_to_its_file() {
    check(&[
        // Observed, e06 and e11: `linkish` is `[[Pumice]]`.
        ("linkish", "[[Pumice]]"),
        ("linkish.toString()", "[[Pumice]]"),
        ("linkish.asFile()", "Pumice.md"),
        ("linkish.asFile().name", "Pumice"),
        ("linkish.length", "6"),
        ("linkish.isTruthy()", "true"),
    ]);

    // `unprobed`: a target the host cannot resolve is null, and null absorbs
    // from there.
    let mut ghosted = silica();
    ghosted.properties.insert(
        "linkish".into(),
        Value::Link(LinkValue {
            path: "Ghost".into(),
            display: None,
        }),
    );
    let ctx = SimpleContext::new().with_file(ghosted);
    assert_eq!(render("linkish.asFile()", &ctx), "null");
    assert_eq!(render("linkish.asFile().name", &ctx), "null");
}

#[test]
fn linksto_takes_a_file_and_nothing_else() {
    check(&[
        ("linkish.linksTo(linkish.asFile())", "true"),
        ("linkish.linksTo(file)", "false"),
        // Observed, e14.
        (
            "linkish.linksTo()",
            "Cannot call function \"linksTo\", not enough arguments.",
        ),
        (
            "linkish.linksTo(/zz/)",
            "Type error in \"linksTo\", parameter \"file\". Expected File, given RegExp.",
        ),
        (
            "linkish.linksTo(/zz/, /zz/)",
            "Type error in \"linksTo\", parameter \"file\". Expected File, given RegExp.",
        ),
        (
            "linkish.linksTo(/zz/, /zz/, /zz/)",
            "Type error in \"linksTo\", parameter \"file\". Expected File, given RegExp.",
        ),
        // `unprobed`: the signature names File alone, so a String is refused
        // the same way — unlike `hasLink`, whose signature lists three types.
        (
            "linkish.linksTo(\"Pumice\")",
            "Type error in \"linksTo\", parameter \"file\". Expected File, given String.",
        ),
    ]);
}

#[test]
fn a_link_is_a_string_receiver() {
    // Observed, e14: everything but `asFile()` and `linksTo()` on a Link is
    // the String set, working on the link's text. These answers come from
    // `funcs::text`, not from `funcs::file`.
    check(&[
        ("linkish.lower()", "pumice"),
        ("linkish.title()", "Pumice"),
        ("linkish.reverse()", "ecimuP"),
        ("linkish.isEmpty()", "false"),
        ("linkish.contains(\"umi\")", "true"),
        ("linkish.startsWith(\"Pum\")", "true"),
        ("linkish.endsWith(\"ice\")", "true"),
        (
            "linkish.contains()",
            "Cannot call function \"contains\", not enough arguments.",
        ),
        (
            "linkish.contains(/zz/)",
            "Type error in \"contains\", parameter \"value\". Expected String, given RegExp.",
        ),
    ]);
}

// --- the refusals, verbatim -------------------------------------------------

#[test]
fn the_refusals_are_obsidians_own_strings() {
    // Observed, e14: the minimum arity first, then the parameter's type —
    // two and three arguments are still the type error, never *too many*.
    check(&[
        (
            "file.hasLink()",
            "Cannot call function \"hasLink\", not enough arguments.",
        ),
        (
            "file.hasLink(/zz/)",
            "Type error in \"hasLink\", parameter \"dest\". Expected File, given RegExp.",
        ),
        (
            "file.hasLink(/zz/, /zz/)",
            "Type error in \"hasLink\", parameter \"dest\". Expected File, given RegExp.",
        ),
        (
            "file.hasLink(/zz/, /zz/, /zz/)",
            "Type error in \"hasLink\", parameter \"dest\". Expected File, given RegExp.",
        ),
        (
            "file.hasProperty()",
            "Cannot call function \"hasProperty\", not enough arguments.",
        ),
        (
            "file.hasProperty(/zz/)",
            "Type error in \"hasProperty\", parameter \"property\". Expected String, given RegExp.",
        ),
        (
            "file.hasProperty(/zz/, /zz/)",
            "Type error in \"hasProperty\", parameter \"property\". Expected String, given RegExp.",
        ),
        (
            "file.hasProperty(/zz/, /zz/, /zz/)",
            "Type error in \"hasProperty\", parameter \"property\". Expected String, given RegExp.",
        ),
        (
            "file.inFolder()",
            "Cannot call function \"inFolder\", not enough arguments.",
        ),
        (
            "file.inFolder(/zz/)",
            "Type error in \"inFolder\", parameter \"folder\". Expected String, given RegExp.",
        ),
        (
            "file.inFolder(/zz/, /zz/)",
            "Type error in \"inFolder\", parameter \"folder\". Expected String, given RegExp.",
        ),
        (
            "file.inFolder(/zz/, /zz/, /zz/)",
            "Type error in \"inFolder\", parameter \"folder\". Expected String, given RegExp.",
        ),
        // `unprobed`: the maximum is reached only once the declared parameter
        // has type-checked, which is the order `funcs::number` records for
        // `toFixed` and `funcs::text` for `contains`.
        (
            "file.hasProperty(\"status\", \"status\")",
            "Cannot call function \"hasProperty\", too many arguments.",
        ),
        (
            "file.inFolder(\"/\", \"/\")",
            "Cannot call function \"inFolder\", too many arguments.",
        ),
        (
            "file.asLink(\"a\", \"b\")",
            "Cannot call function \"asLink\", too many arguments.",
        ),
        (
            "linkish.asFile(1)",
            "Cannot call function \"asFile\", too many arguments.",
        ),
    ]);
}

// --- null absorption, which the dispatcher owns -----------------------------

#[test]
fn a_row_without_a_file_answers_null() {
    // The dispatcher's rule, checked here because `file` is the one ident
    // that can be missing: a view can be asked for a row that has no file.
    let ctx = SimpleContext::new();
    for expr in [
        "file.name",
        "file.path",
        "file.tags",
        "file.hasTag(\"rock\")",
        "file.asLink()",
        "file.nosuch",
    ] {
        assert_eq!(render(expr, &ctx), "null", "{expr}");
    }
    assert_eq!(render("file.isEmpty()", &ctx), "true");
    assert_eq!(render("file.isTruthy()", &ctx), "false");
    assert_eq!(render("file.toString()", &ctx), "null");
}

// --- `this` is the base file ------------------------------------------------

#[test]
fn this_is_the_base_file_and_its_name_keeps_the_extension() {
    // Observed, e11 and e17: `this.file.name` is `Main.base` where the row's
    // own `file.name` is `Silica`. That contrast is the whole evidence for
    // what `name` holds — a file name with the `.md` dropped and any other
    // extension kept.
    //
    // Only `this.file.*` is asserted here. What `this` itself is beyond that
    // is not this module's to say and the run does not settle it: the
    // completion list for `this.` is the File *functions* plus `file`,
    // `note`, `formula`, `this` and the vault's frontmatter names, and none
    // of the File properties, so `this` is not simply a File value.
    check(&[
        ("this.file.name", "Main.base"),
        ("this.file.path", "Main.base"),
        ("this.file.ext", "base"),
    ]);
}
