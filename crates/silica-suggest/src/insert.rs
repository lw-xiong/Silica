//! The text accepting a suggestion writes.
//!
//! A link's path part is `fileToLinktext` in the vault's `newLinkFormat`,
//! so the crate reuses [`silica_cache::links::link_path`] rather than
//! formatting a second kind of link. The three accept keys differ, and the
//! difference is the point: Enter finishes the link, Tab leaves the cursor
//! inside it so a `#heading` can follow, and Shift+Enter links the text as
//! typed. See
//! [[Specification#Observed autocompletion behavior, Obsidian 1.14.4]].

use crate::candidates::{Item, Vault};
use crate::trigger::{Kind, Triggered};
use silica_cache::links::{basename, link_path, LinkFormat};
use silica_formats::FileKind;
use silica_markdown::parse_linktext;

/// Which key accepted the suggestion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accept {
    /// Enter: the whole link, and the cursor after it.
    Whole,
    /// Tab: the same link with the cursor left inside the brackets.
    Inside,
    /// Shift+Enter: ignore the selection and link the query as typed.
    Typed,
}

/// `newLinkFormat` and `useMarkdownLinks` from `app.json`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    pub format: LinkFormat,
    pub markdown_links: bool,
}

/// An edit that replaces the trigger's range, plus `consume` bytes after
/// it, with `text`, leaving the cursor `cursor` bytes into `text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Insertion {
    pub text: String,
    pub cursor: usize,
    /// A closing `]]` already after the cursor, which the new link replaces
    /// rather than doubling.
    pub consume: usize,
}

/// Obsidian encodes only spaces in a markdown link destination.
fn encode(path: &str) -> String {
    path.replace(' ', "%20")
}

/// The text `item` writes for `t`, and where the cursor lands.
pub fn insert(
    t: &Triggered<'_>,
    item: &Item,
    accept: Accept,
    v: &Vault<'_>,
    opts: &Options,
) -> Insertion {
    match t.kind {
        Kind::Tag => tag(t, item, accept),
        Kind::Link | Kind::Embed => link(t, item, accept, v, opts),
    }
}

/// A tag is written with a trailing space on Enter and without one on Tab.
fn tag(t: &Triggered<'_>, item: &Item, accept: Accept) -> Insertion {
    let name = match item {
        Item::Tag { tag } => tag.clone(),
        _ => t.query.to_string(),
    };
    let text = match accept {
        Accept::Whole => format!("#{name} "),
        _ => format!("#{name}"),
    };
    Insertion {
        cursor: text.len(),
        text,
        consume: 0,
    }
}

/// The path, subpath and display text a link to `item` is written with.
struct Parts {
    /// The path part, in the vault's `newLinkFormat`.
    path: String,
    /// `#Heading`, if any.
    subpath: String,
    /// The text after `|`, or the markdown link's text.
    display: Option<String>,
    /// Whether a file stands behind the link at all.
    resolved: bool,
    /// Whether the target is a note (`.md` is kept in a markdown link).
    markdown: bool,
}

fn parts(t: &Triggered<'_>, item: &Item, v: &Vault<'_>, opts: &Options) -> Parts {
    let resolver = v.cache.resolver();
    let of_file = |path: &str| link_path(resolver, path, v.source, opts.format);
    match item {
        Item::File { path } => {
            let written = of_file(path);
            let markdown = FileKind::from_path(path).is_markdown();
            // The display text is added exactly when the path part carries a
            // folder, and only for notes: `[[Sub/Alpha|Alpha]]`, but
            // `[[../pic.png]]` with none.
            let display = (markdown && written.contains('/')).then(|| basename(path).to_string());
            Parts {
                path: written,
                subpath: String::new(),
                display,
                resolved: true,
                markdown,
            }
        }
        Item::Alias { path, alias } => Parts {
            path: of_file(path),
            subpath: String::new(),
            display: Some(alias.clone()),
            resolved: true,
            markdown: FileKind::from_path(path).is_markdown(),
        },
        // A heading pick is the exception twice over: the path stays exactly
        // as it was typed rather than being reformatted, and it never takes
        // display text. `[[Sub/Gamma#` keeps `Sub/Gamma` even where
        // `shortest` would have written `Gamma`.
        Item::Heading { heading, .. } => Parts {
            path: parse_linktext(t.query).path.to_string(),
            subpath: format!("#{heading}"),
            display: None,
            resolved: true,
            markdown: true,
        },
        // A block pick follows the heading rule for the path, and writes no
        // display text at all: `[](Gamma.md#^id)` in markdown style. A
        // block with no id of its own cannot be linked until one is
        // written, which the caller does before asking for the link.
        Item::Block { id, .. } => Parts {
            path: parse_linktext(t.query).path.to_string(),
            subpath: match id {
                crate::BlockId::Existing(id) => format!("#^{id}"),
                crate::BlockId::New { .. } => "#^".to_string(),
            },
            display: None,
            resolved: true,
            markdown: true,
        },
        // Nothing resolved: the query is linked as it was typed. A `|` in it
        // still splits, so `[[Alpha|disp` keeps `disp` as the display text
        // and links `Alpha` even though no file was picked.
        Item::Tag { .. } | Item::Literal => {
            let parsed = parse_linktext(t.query);
            Parts {
                path: parsed.path.to_string(),
                subpath: String::new(),
                display: parsed.alias.map(str::to_string),
                resolved: false,
                markdown: false,
            }
        }
    }
}

fn link(
    t: &Triggered<'_>,
    item: &Item,
    accept: Accept,
    v: &Vault<'_>,
    opts: &Options,
) -> Insertion {
    let bang = if t.kind.is_embed() { "!" } else { "" };
    // Shift+Enter links the text as typed, whatever is selected.
    let typed = accept == Accept::Typed;
    let p = if typed {
        Parts {
            path: t.query.to_string(),
            subpath: String::new(),
            display: None,
            resolved: false,
            markdown: false,
        }
    } else {
        parts(t, item, v, opts)
    };
    let (text, cursor) = if opts.markdown_links {
        // `[Beta Note](Beta%20Note.md)`: the text is the basename, the alias
        // or the heading, and a pick with no file behind it writes none.
        let shown = match (&p.display, &p.resolved, item) {
            (Some(d), _, _) => d.clone(),
            (None, true, Item::Heading { heading, .. }) => heading.clone(),
            // A block reference has no text to show; the cursor lands
            // between the brackets, which is the part left to fill in.
            (None, _, Item::Block { .. }) if !typed => String::new(),
            (None, true, _) => basename(&p.path).to_string(),
            (None, false, _) => String::new(),
        };
        let mut dest = p.path.clone();
        if p.markdown && p.resolved {
            dest.push_str(".md");
        }
        dest.push_str(&p.subpath);
        let text = format!("{bang}[{shown}]({})", encode(&dest));
        // There is nowhere to stand inside a markdown link, so Tab lands
        // where Enter does — except with no text to show, where the cursor
        // goes between the brackets, that being the part left to fill in.
        let cursor = if shown.is_empty() {
            bang.len() + 1
        } else {
            text.len()
        };
        (text, cursor)
    } else {
        let display = p.display.map(|d| format!("|{d}")).unwrap_or_default();
        let body = format!("{}{}{}", p.path, p.subpath, display);
        let text = format!("{bang}[[{body}]]");
        let cursor = match accept {
            Accept::Inside => text.len() - 2,
            _ => text.len(),
        };
        (text, cursor)
    };
    // Accepting inside `[[be|]]` must not leave four brackets.
    let consume = if t.after.starts_with("]]") { 2 } else { 0 };
    Insertion {
        text,
        cursor,
        consume,
    }
}
