//! The vault exported as a static HTML site: every note rendered through the
//! same parser the reading view uses, links rewritten to the exported pages,
//! assets copied beside them.
//!
//! The point is publishing without a plugin: one command writes a folder a
//! browser (or a static host) can serve straight away. It is a snapshot, not
//! a sync — run it again and the folder is rebuilt.
//!
//! Links: a resolved wikilink becomes a relative URL to the exported page —
//! relative, never root-relative, so the folder works opened straight from
//! disk with no server. An unresolved link loses its `href` (the anchor
//! stays as text, `data-href` kept for round-tripping); an external link is
//! left exactly as the renderer wrote it. An embed of a *note* renders
//! nothing in the reading view's HTML either, so it is left alone too — the
//! ceiling here is images, audio and video.

use silica_formats::FileKind;
use silica_vault::VaultFile;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

/// What one export pass did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteReport {
    pub notes: usize,
    pub links: usize,
    pub unresolved: usize,
    pub output: PathBuf,
}

/// Directories of the vault that are never published.
const SKIPPED_DIRS: [&str; 3] = [".obsidian", ".trash", ".git"];

/// Whether a vault path lives in a directory that is never published, at
/// any depth.
fn skipped_rel(rel: &str) -> bool {
    rel.replace('\\', "/")
        .to_lowercase()
        .split('/')
        .any(|seg| SKIPPED_DIRS.contains(&seg))
}

/// The notes to publish: markdown, minus the directories a vault keeps its
/// own house in — Obsidian files, trashed notes and git metadata.
fn publishable_notes(files: &[VaultFile]) -> Vec<&VaultFile> {
    files
        .iter()
        .filter(|f| f.kind == FileKind::Markdown && !skipped_rel(&f.path))
        .collect()
}

/// Whether `out` is a safe place to write a site: outside the vault (a
/// folder inside it would be indexed and copied on every pass), and either
/// new or holding nothing but what an earlier pass wrote.
fn output_is_safe(out: &Path, root: &Path) -> Result<(), String> {
    if out == root || out.starts_with(root) {
        return Err(format!(
            "{} is inside the vault — pick a folder outside it",
            out.display()
        ));
    }
    if !out.exists() {
        return Ok(());
    }
    let foreign: Vec<String> = std::fs::read_dir(out)
        .map_err(|e| format!("Cannot read {}: {e}", out.display()))?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| !(name == "index.html" || name == "style.css" || name.ends_with(".html")))
        .collect();
    if !foreign.is_empty() {
        return Err(format!(
            "{} holds files that are not site output ({}); pick an empty folder or a previous export",
            out.display(),
            foreign[0]
        ));
    }
    Ok(())
}

/// The raw HTML a note's body must not carry into a published site: active
/// content survives the renderer untouched, and an export turns one vault
/// into a site every reader of it visits.
///
/// This is a trust boundary — a note's raw HTML reaches here verbatim — so
/// the match is case-insensitive (a browser runs `<SCRIPT>` like `<script>`)
/// and the pass repeats until no opener survives: one pass leaves stitched
/// pieces behind (`<scr<script>x</script>ipt>` emits `<scr` + `ipt>`, a
/// whole live `<script>` in the output).
fn strip_active(html: &str) -> String {
let mut out = html.to_string();
loop {
    let stripped = strip_active_once(&out);
    let done = stripped == out;
    out = stripped;
    if done {
        return strip_meta_refresh(&strip_event_attributes(&out));
    }
}
}

/// One sweep: every `<script`/`<iframe` element found in `html`, dropped
/// case-insensitively. `to_ascii_lowercase` maps only A-Z, so the copy has
/// the same byte length and indices still slice the original.
fn strip_active_once(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    loop {
        let lower = rest.to_ascii_lowercase();
        let open = ["<script", "<iframe", "<object", "<embed", "<applet", "<style", "<base"]
            .iter()
            .filter_map(|tag| lower.find(tag).map(|at| (at, *tag)))
            .min_by_key(|(at, _)| *at);
        let Some((at, tag)) = open else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..at]);
        // * The needle drops the leading `<` of the tag: `</<script`
        //   matches nothing, so the element's content would survive as
        //   visible (and patchable) text instead of disappearing.
        let close = format!("</{}", &tag[1..]);
        // ! An unterminated element drops the tag, not the rest of the
        //   page: a truncated render must not swallow the note. Skipping
        //   past the opening tag guarantees the walk makes progress.
        let after = lower[at..]
            .find(&close)
            .map(|i| at + i + close.len())
            .and_then(|i| lower[i..].find('>').map(|j| i + j + 1))
            .or_else(|| lower[at..].find('>').map(|i| at + i + 1))
            .unwrap_or(rest.len());
        rest = &rest[after..];
    }
}

/// Every tag that survived the element pass is walked again and its
/// attributes cut, span by span, so untouched text keeps its exact bytes:
/// the ordinary-links contract is byte equality, not "close enough".
///
/// Two things go: any `on*` attribute (every one of them is an event
/// handler) and any URL-valued attribute whose value a browser would
/// resolve to an active scheme. A `<` that opens no tag (`a < b`, `<!`,
/// `</`) is copied as text, so a stray angle bracket never eats the note.
fn strip_event_attributes(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(at) = rest.find('<') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let opener = rest.as_bytes().get(1);
        if !opener.is_some_and(|b| b.is_ascii_alphabetic()) {
            let end = rest.find('>').map(|i| i + 1).unwrap_or(1.min(rest.len()));
            out.push_str(&rest[..end]);
            rest = &rest[end.min(rest.len())..];
            continue;
        }
        match cut_tag_attributes(rest) {
            Some((end, kept)) => {
                out.push_str(&kept);
                rest = &rest[end..];
            }
            // No '>' outside quotes: the '<' was text, keep the byte.
            None => {
                out.push('<');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Walks one opening tag (`rest` starts at its `<`), returning the byte
/// offset just past its closing `>` and the tag with its active
/// attributes cut, or `None` when no closing `>` sits outside a quoted
/// value.
fn cut_tag_attributes(rest: &str) -> Option<(usize, String)> {
    let bytes = rest.as_bytes();
    let mut i = 1;
    let mut cuts: Vec<(usize, usize)> = Vec::new();
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        match bytes.get(i) {
            None => return None,
            Some(b'>') => {
                i += 1;
                break;
            }
            // A trailing `/` (self-closing) closes the tag once its `>` lands.
            Some(b'/') => {
                i += 1;
                continue;
            }
            _ => {}
        }
        let attr_start = i;
        let name_start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && !matches!(bytes[i], b'=' | b'/' | b'>')
        {
            i += 1;
        }
        let name = &rest[name_start..i];
        let mut value = String::new();
        if bytes.get(i) == Some(&b'=') {
            i += 1;
            match bytes.get(i) {
                Some(q @ (b'"' | b'\'')) => {
                    let quote = *q;
                    i += 1;
                    let val_start = i;
                    while i < bytes.len() && bytes[i] != quote {
                        i += 1;
                    }
                    value.push_str(&rest[val_start..i.min(bytes.len())]);
                    if i < bytes.len() {
                        i += 1;
                    }
                }
                _ => {
                    let val_start = i;
                    while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                        i += 1;
                    }
                    value.push_str(&rest[val_start..i]);
                }
            }
        }
        let attr_end = i;
        if name.len() > 2 && name.len() <= 24 && name.to_ascii_lowercase().starts_with("on")
            || is_active_url(&value)
            || name.eq_ignore_ascii_case("style")
        {
            cuts.push((attr_start, attr_end));
        }
    }
    let mut kept = String::with_capacity(rest.len());
    let mut last = 0;
    for (from, to) in cuts {
        kept.push_str(&rest[last..from]);
        last = to;
    }
    kept.push_str(&rest[last..i]);
    Some((i, kept))
}

/// Removes `<meta http-equiv="refresh" ...>` tags, which a browser treats as
/// a timed redirect (open redirect / phishing vector). Case-insensitive on
/// both the attribute name and the `refresh` value, quoted or unquoted.
fn strip_meta_refresh(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(at) = rest.find("<meta") {
        let lower = rest[at..].to_ascii_lowercase();
        if !lower.starts_with("<meta") {
            out.push_str(&rest[..at + 1]);
            rest = &rest[at + 1..];
            continue;
        }
        // Find the end of the tag (outside quotes).
        let bytes = &rest.as_bytes()[at..];
        let mut i = 5; // skip "<meta"
        let mut in_quote: Option<u8> = None;
        let mut end = None;
        while i < bytes.len() {
            match (in_quote, bytes[i]) {
                (Some(q), b) if b == q => in_quote = None,
                (None, b'"' | b'\'') => in_quote = Some(bytes[i]),
                (None, b'>') => {
                    end = Some(at + i + 1);
                    break;
                }
                _ => {}
            }
            i += 1;
        }
        let Some(end) = end else {
            out.push_str(&rest[at..]);
            break;
        };
        let tag = &rest[at..end];
        let tag_lower = tag.to_ascii_lowercase();
        let is_refresh = tag_lower.contains("http-equiv")
            && tag_lower.contains("refresh");
        if is_refresh {
            // Drop the whole tag.
        } else {
            out.push_str(tag);
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// Whether a browser resolving `value` as a URL lands on a scheme that
/// runs code. The check runs on the value a browser would see: numeric
/// character references decoded (`&#106;avascript:` is `javascript:`),
/// `&colon;` spelled out, case and the ASCII whitespace/control bytes a
/// URL parser drops (`java\tscript:`) normalized away.
fn is_active_url(value: &str) -> bool {
    let normalized = normalize_url(value);
    normalized.starts_with("javascript:")
        || normalized.starts_with("vbscript:")
        || normalized.starts_with("data:text/html")
        || normalized.starts_with("data:image/svg+xml")
}

/// Decodes the entity forms a URL parser accepts, lowercases, and drops
/// the bytes browsers ignore inside a scheme.
fn normalize_url(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '&' {
            if !c.is_ascii_whitespace() && !c.is_ascii_control() {
                out.push(c.to_ascii_lowercase());
            }
            continue;
        }
        let mut refr = String::new();
        while let Some(&next) = chars.peek() {
            if next == ';' {
                chars.next();
                break;
            }
            if refr.len() > 10 {
                break;
            }
            refr.push(chars.next().unwrap());
        }
        match refr.as_str() {
            "colon" => out.push(':'),
            "Tab" | "NewLine" | "sol" => out.push(' '),
            other => {
                let hex = other
                    .strip_prefix("#x")
                    .or_else(|| other.strip_prefix("#X"))
                    .and_then(|h| u32::from_str_radix(h, 16).ok());
                let dec = other.strip_prefix('#').and_then(|d| d.parse::<u32>().ok());
                if let Some(cp) = hex.or(dec).and_then(char::from_u32) {
                    out.push(cp.to_ascii_lowercase());
                } else {
                    out.push('&');
                    out.push_str(other);
                    out.push(';');
                }
            }
        }
    }
    out
}

/// How many bytes of a single note the export will read before refusing.
/// A published site must never carry a multi-gigabyte note; 10 MB is ample
/// for any reasonable note and bounds the render's memory.
pub const EXPORT_MAX_BYTES: u64 = 10 << 20;

/// Exports the vault at `root` (whose files are `files`) into `out`,
/// replacing whatever an earlier pass wrote there.
pub fn export(root: &Path, files: &[VaultFile], out: &Path) -> Result<SiteReport, String> {
    let notes: Vec<&VaultFile> = publishable_notes(files);
    if notes.is_empty() {
        return Err("The vault has no notes to export.".into());
    }
    output_is_safe(out, root)?;
    let map = link_map(&notes);
    let assets = asset_paths(files);
    std::fs::create_dir_all(out).map_err(|e| format!("Cannot create {}: {e}", out.display()))?;

    let mut report = SiteReport {
        notes: 0,
        links: 0,
        unresolved: 0,
        output: out.to_path_buf(),
    };

    // * Every note renders to `out/<same folders>/<stem>.html`. A page's
    //   depth is its folder's, which is what relative URLs are built from.
    for note in &notes {
        let rel = note.path.replace('\\', "/");
        let depth = rel.matches('/').count();
        let text = silica_vault::read_verified(root, Path::new(&rel), EXPORT_MAX_BYTES)
            .map_err(|e| format!("Cannot read {rel}: {e}"))?;
        let body = rewrite(&render(&text), &map, &assets, depth);
        report.links += body.links;
        report.unresolved += body.unresolved;
        write_page(out, &page_path(&rel), &title_of(&rel), depth, &body.html)?;
        report.notes += 1;
    }

    copy_assets(root, files, out)?;
    std::fs::write(out.join("style.css"), STYLE)
        .map_err(|e| format!("Cannot write style.css: {e}"))?;

    // * `index.html` is the note called README, index or Home when there is
    //   one, else the first note — rendered like any other, at depth 0.
    let home = ["README.md", "index.md", "Home.md"]
        .iter()
        .find_map(|name| notes.iter().find(|n| n.path.eq_ignore_ascii_case(name)))
        .unwrap_or(&notes[0]);
    let text = silica_vault::read_verified(root, Path::new(&home.path), EXPORT_MAX_BYTES)
        .map_err(|e| format!("Cannot read {}: {e}", home.path))?;
    let body = rewrite(&render(&text), &map, &assets, 0);
    write_page(out, "index.html", &title_of(&home.path), 0, &body.html)?;
    Ok(report)
}

/// Parses and renders one note the way the reading view would, minus the
/// active content a published site must never carry.
fn render(text: &str) -> String {
    let doc = silica_markdown::parse(text);
    strip_active(&silica_markdown::html::render_html(&doc))
}

/// A page's `<title>`: its name without the extension.
fn title_of(rel: &str) -> String {
    rel.rsplit('/')
        .next()
        .unwrap_or(rel)
        .strip_suffix(".md")
        .unwrap_or_else(|| rel.rsplit('/').next().unwrap_or(rel))
        .to_string()
}

/// Wraps a rendered body in the one document shape every page shares.
fn write_page(out: &Path, name: &str, title: &str, depth: usize, body: &str) -> Result<(), String> {
    let escaped_title = escape_title(title);
    let page = format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{escaped_title}</title>\n<link rel=\"stylesheet\" href=\"{}style.css\">\n\
         </head>\n<body>\n{body}\n</body>\n</html>\n",
        "../".repeat(depth),
    );
    let path = out.join(name);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    }
    std::fs::write(&path, page).map_err(|e| format!("Cannot write {}: {e}", path.display()))
}

/// The link table: every way a note can be named — full path with and
/// without the extension, bare stem with and without — lower-cased, mapped
/// to the note's vault path. Shorter paths claim a shared name first, which
/// is Obsidian's shortest-path rule for ambiguous names.
fn link_map(notes: &[&VaultFile]) -> HashMap<String, String> {
    let mut sorted: Vec<&str> = notes.iter().map(|n| n.path.as_str()).collect();
    sorted.sort_by_key(|p| (p.len(), *p));
    let mut map = HashMap::new();
    for path in sorted {
        let lower = path.to_lowercase();
        let without_ext = lower.strip_suffix(".md").unwrap_or(&lower).to_string();
        let stem = without_ext
            .rsplit('/')
            .next()
            .unwrap_or(&without_ext)
            .to_string();
        for key in [
            &lower,
            without_ext.as_str(),
            stem.as_str(),
            &format!("{stem}.md"),
        ] {
            map.entry(key.to_string())
                .or_insert_with(|| path.to_string());
        }
    }
    map
}

/// The same table for every publishable asset: lower-cased vault path to
/// itself, so an embed's `src` can be checked and kept honest.
fn asset_paths(files: &[VaultFile]) -> HashMap<String, String> {
    files
        .iter()
        .filter(|f| f.kind != FileKind::Markdown)
        .map(|f| {
            (
                f.path.replace('\\', "/").to_lowercase(),
                f.path.replace('\\', "/"),
            )
        })
        .collect()
}

/// The page path a note exports to: same folders, `.md` becomes `.html`.
fn page_path(rel: &str) -> String {
    let without = rel.strip_suffix(".md").unwrap_or(rel);
    format!("{without}.html")
}

/// Resolves a link's text to the vault path it names, or `None`. The subpath
/// (`#heading`, `#^block`) never takes part; the name may be spelled with the
/// extension or without, in any case.
fn resolve<'m>(link: &str, map: &'m HashMap<String, String>) -> Option<&'m str> {
    let name = link.split('#').next().unwrap_or(link).trim();
    if name.is_empty() {
        return None;
    }
    let lower = name.trim_end_matches(".md").to_lowercase();
    map.get(&lower).map(|p| p.as_str())
}

/// The three characters that end a title element or start an entity.
/// `escape_html` in the renderer is crate-private, and a title never
/// needs more than these three.
fn escape_title(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    for ch in title.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

/// The linker's attribute values arrive HTML-escaped (the renderer escapes
/// the link text), so `A & B.md` reaches here as `A &amp; B.md`. Undoing
/// the five named references is enough for the vault's legal file-name
/// characters; anything else is left as written.
fn unescape_attr(value: &str) -> String {
    if !value.contains('&') {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let entity = [
            ("amp;", "&"),
            ("lt;", "<"),
            ("gt;", ">"),
            ("quot;", "\""),
            ("#39;", "'"),
        ]
        .iter()
        .find(|(name, _)| after.starts_with(name));
        match entity {
            Some((name, ch)) => {
                out.push_str(ch);
                rest = &after[name.len()..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The URL one exported page or asset is reached by from another: the
/// relative walk from `from_depth` (the number of folder components above
/// the page) to the target's path inside the site.
fn rel_url(from_depth: usize, to: &str) -> String {
    let mut url = "../".repeat(from_depth);
    url.push_str(to);
    url
}

/// What rewriting one page did.
struct Rewritten {
    html: String,
    links: usize,
    unresolved: usize,
}

/// Rewrites the internal links and embeds of one rendered page.
///
/// The renderer emits `class="internal-link"` anchors whose `href` is the
/// link text as written and never a working URL, and `class="internal-embed"`
/// spans whose `src` is the resource path as written. The pass finds each
/// such tag, resolves its `data-href` (anchors) or `src` (embeds of a copied
/// asset), and puts the page's relative URL in its place. The renderer's
/// `target="_blank" rel="noopener nofollow"` — right for the app, wrong for
/// a site's own pages — goes with the rewritten `href`. An unresolved link
/// loses the `href` and keeps the text; an embed of a note (which the
/// renderer draws as an empty span anyway) is left as it is.
fn rewrite(
    html: &str,
    map: &HashMap<String, String>,
    assets: &HashMap<String, String>,
    depth: usize,
) -> Rewritten {
    let mut out = String::with_capacity(html.len());
    let mut links = 0usize;
    let mut unresolved = 0usize;
    let mut rest = html;
    while let Some(at) = next_tag(rest) {
        let which = if rest[at..].starts_with("class=\"internal-link\"") {
            "internal-link"
        } else {
            "internal-embed"
        };
        let attr = if which == "internal-link" {
            "data-href"
        } else {
            "src"
        };
        let tag_start = rest[..at].rfind('<').unwrap_or(at);
        let tag_end = at + rest[at..].find('>').map(|i| i + 1).unwrap_or(0);
        let value = rest[tag_start..tag_end]
            .find(&format!("{attr}=\""))
            .map(|i| {
                let at = i + attr.len() + 2;
                let end = rest[tag_start + at..]
                    .find('"')
                    .map(|e| tag_start + at + e)
                    .unwrap_or(tag_end - 1);
                rest[tag_start + at..end].to_string()
            });
        let raw = value.as_deref().unwrap_or_default();
        let unescaped = unescape_attr(raw);
        let value = Some(unescaped);
        let hit = value.as_deref().and_then(|v| match which {
            "internal-link" => resolve(v, map).map(page_path),
            _ => assets.get(&v.to_lowercase()).cloned(),
        });
        let tag = match (value, hit) {
            (Some(_), Some(target)) => {
                links += if which == "internal-link" { 1 } else { 0 };
                let tag = &rest[tag_start..tag_end];
                let tag = replace_attr(
                    tag,
                    if which == "internal-link" {
                        "href"
                    } else {
                        "src"
                    },
                    &rel_url(depth, &target),
                );
                if which == "internal-link" {
                    tag.replace(" target=\"_blank\" rel=\"noopener nofollow\"", "")
                } else {
                    tag
                }
            }
            (Some(_), None) if which == "internal-link" => {
                unresolved += 1;
                remove_attr(&rest[tag_start..tag_end], "href")
                    .replace(" target=\"_blank\" rel=\"noopener nofollow\"", "")
            }
            _ => {
                // * An embed with no resolvable source, or a tag the pass
                //   cannot read, is copied through untouched.
                rest[tag_start..tag_end].to_string()
            }
        };
        out.push_str(&rest[..tag_start]);
        out.push_str(&tag);
        rest = &rest[tag_end..];
    }
    out.push_str(rest);
    Rewritten {
        html: out,
        links,
        unresolved,
    }
}

/// Where the next rewritable tag — an `internal-link` anchor or an
/// `internal-embed` span — starts in `rest`, whichever comes first.
fn next_tag(rest: &str) -> Option<usize> {
    let link = rest.find("class=\"internal-link\"");
    let embed = rest.find("class=\"internal-embed\"");
    match (link, embed) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, None) => a,
        (None, b) => b,
    }
}

/// Replaces one attribute's value inside a tag, keeping the quotes.
/// The needle is space-prefixed: a bare `href="` also matches inside
/// `data-href="`, which would rewrite the wrong attribute.
fn replace_attr(tag: &str, name: &str, value: &str) -> String {
    let needle = format!(" {name}=\"");
    match tag.find(&needle) {
        Some(at) => {
            let value_at = at + needle.len();
            let end = tag[value_at..]
                .find('"')
                .map(|i| value_at + i)
                .unwrap_or(value_at);
            format!("{}{}{}", &tag[..value_at], value, &tag[end..])
        }
        None => tag.to_string(),
    }
}

/// Removes one attribute and its value from a tag.
fn remove_attr(tag: &str, name: &str) -> String {
    let needle = format!(" {name}=\"");
    match tag.find(&needle) {
        Some(at) => {
            let value_at = at + needle.len();
            let end = tag[value_at..]
                .find('"')
                .map(|i| value_at + i + 1)
                .unwrap_or(value_at);
            format!("{}{}", &tag[..at], &tag[end..])
        }
        None => tag.to_string(),
    }
}

/// Copies every non-note file — the attachments a page may embed — keeping
/// the folder structure. The config folders stay out.
fn copy_assets(root: &Path, files: &[VaultFile], out: &Path) -> Result<(), String> {
    for file in files {
        if file.kind == FileKind::Markdown {
            continue;
        }
        let rel = file.path.replace('\\', "/");
        if SKIPPED_DIRS
            .iter()
            .any(|d| rel.starts_with(&format!("{d}/")))
        {
            continue;
        }
        let from = root.join(&rel);
        let to = out.join(&rel);
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
        }
        std::fs::copy(&from, &to).map_err(|e| format!("Cannot copy {rel}: {e}"))?;
    }
    Ok(())
}

/// The stylesheet every page links. Deliberately small and quiet: a single
/// column, the code and quote shapes, nothing a theme would own.
const STYLE: &str = ":root { color-scheme: light dark; }
body { font: 16px/1.6 -apple-system, 'Segoe UI', Roboto, sans-serif; max-width: 44rem; margin: 2rem auto; padding: 0 1rem; }
h1, h2, h3 { line-height: 1.25; }
a { color: #4a6da7; }
img, video { max-width: 100%; height: auto; }
pre { background: #f4f4f4; padding: 0.75rem 1rem; border-radius: 6px; overflow-x: auto; }
@media (prefers-color-scheme: dark) { pre { background: #1f1f23; } body { background: #141417; color: #e6e6e6; } }
blockquote { border-inline-start: 3px solid #8888; margin-inline: 0; padding-inline-start: 1rem; color: inherit; opacity: 0.85; }
table { border-collapse: collapse; }
th, td { border: 1px solid #8884; padding: 0.3rem 0.6rem; }
a.internal-link[data-href]:not([href]) { color: #a33; }
";

#[cfg(test)]
mod tests {
    use super::*;

    fn note(path: &str) -> VaultFile {
        VaultFile {
            path: path.to_string(),
            kind: FileKind::Markdown,
            size: 0,
            mtime: 0,
            ctime: 0,
        }
    }

    const ANCHOR: &str = "<a data-href=\"{link}\" href=\"{link}\" class=\"internal-link\" target=\"_blank\" rel=\"noopener nofollow\">T</a>";

    fn anchor(link: &str) -> String {
        ANCHOR.replace("{link}", link)
    }

    #[test]
    fn a_link_rewrite_replaces_href_and_never_its_data_href() {
        let map = link_map(&[&note("Target.md")]);
        let empty = HashMap::new();
        let r = rewrite(&anchor("Target"), &map, &empty, 0);
        // The real href carries the URL; data-href keeps the link text.
        assert!(r.html.contains(" href=\"Target.html\""), "{}", r.html);
        assert!(r.html.contains("data-href=\"Target\""), "{}", r.html);
        assert!(!r.html.contains("data-href=\"Target.html\""), "{}", r.html);
    }

    #[test]
    fn the_link_map_prefers_the_shortest_path_for_a_shared_name() {
        let deep = note("Sub/Deep/Plan.md");
        let top = note("Plan.md");
        let map = link_map(&[&deep, &top]);
        assert_eq!(map["plan"], "Plan.md");
        assert_eq!(map["sub/deep/plan"], "Sub/Deep/Plan.md");
        // The extension spelled out resolves too, any case.
        assert_eq!(map["plan.md"], "Plan.md");
    }

    #[test]
    fn a_resolved_link_points_at_its_page_relatively_and_opens_in_place() {
        let map = link_map(&[&note("Target.md"), &note("Sub/Deep/Note.md")]);
        let empty = HashMap::new();
        // From a root page: the target is beside it.
        let r = rewrite(&anchor("Target"), &map, &empty, 0);
        assert_eq!((r.links, r.unresolved), (1, 0));
        assert!(r.html.contains("href=\"Target.html\""), "{}", r.html);
        assert!(!r.html.contains("_blank"), "{}", r.html);
        // From two folders down, the walk climbs twice.
        let r = rewrite(&anchor("Target"), &map, &empty, 2);
        assert!(r.html.contains("href=\"../../Target.html\""), "{}", r.html);
        // A subpath takes part in resolution but never in the URL.
        let r = rewrite(&anchor("Target#Heading"), &map, &empty, 0);
        assert!(r.html.contains("href=\"Target.html\""), "{}", r.html);
    }

    #[test]
    fn an_unresolved_link_loses_its_href_but_keeps_its_text() {
        let map = link_map(&[&note("Target.md")]);
        let empty = HashMap::new();
        let r = rewrite(&anchor("Nowhere"), &map, &empty, 0);
        assert_eq!((r.links, r.unresolved), (0, 1));
        assert!(r.html.contains("data-href=\"Nowhere\""), "{}", r.html);
        assert!(!r.html.contains(" href=\""), "{}", r.html);
        assert!(!r.html.contains("_blank"), "{}", r.html);
        assert!(r.html.contains(">T</a>"), "{}", r.html);
    }

    #[test]
    fn external_links_and_headings_pass_through() {
        let map = link_map(&[&note("Target.md")]);
        let empty = HashMap::new();
        let src = "<a data-tooltip-position=\"top\" aria-label=\"e.com\" rel=\"noopener nofollow\" class=\"external-link\" href=\"https://e.com\" target=\"_blank\">e</a>\n\
                   <h1 data-heading=\"H\">H</h1>\n".to_string()
            + &anchor("Target");
        let r = rewrite(&src, &map, &empty, 0);
        assert!(r.html.contains("href=\"https://e.com\""), "{}", r.html);
        assert!(r.html.contains("data-heading=\"H\""), "{}", r.html);
        assert_eq!((r.links, r.unresolved), (1, 0));
    }

    #[test]
    fn an_embed_of_a_copied_asset_points_at_it_and_a_note_embed_is_left() {
        let map = link_map(&[&note("Target.md")]);
        let mut assets = HashMap::new();
        assets.insert("pic.png".to_string(), "pic.png".to_string());
        let src = "<span alt=\"pic.png\" src=\"pic.png\" class=\"internal-embed\"></span>\n\
                   <span alt=\"Note\" src=\"Note\" class=\"internal-embed\"></span>";
        let r = rewrite(src, &map, &assets, 1);
        assert!(r.html.contains("src=\"../pic.png\""), "{}", r.html);
        // The note embed resolves to nothing publishable and stays as it was.
        assert!(r.html.contains("src=\"Note\""), "{}", r.html);
    }

    #[test]
    fn a_link_named_with_an_ampersand_resolves_through_the_escaped_attr() {
        let map = link_map(&[&note("A & B.md"), &note("Target.md")]);
        let empty = HashMap::new();
        // The renderer escapes the href, so the tag carries the entity.
        let src = anchor("A &amp; B");
        let r = rewrite(&src, &map, &empty, 0);
        assert_eq!((r.links, r.unresolved), (1, 0), "{}", r.html);
        assert!(r.html.contains("href=\"A & B.html\""), "{}", r.html);
        assert!(!r.html.contains("_blank"), "{}", r.html);
    }

    #[test]
    fn the_page_title_escapes_the_three_hostile_characters() {
        assert_eq!(escape_title("A & B"), "A &amp; B");
        assert_eq!(escape_title("<script>"), "&lt;script&gt;");
        assert_eq!(escape_title("plain"), "plain");
        assert_eq!(unescape_attr("A &amp; B"), "A & B");
        assert_eq!(unescape_attr("A & B"), "A & B");
        assert_eq!(unescape_attr("&#39;q&#39;"), "'q'");
    }

    #[test]
    fn a_script_elements_content_goes_with_it() {
        // Dropping only the opening tag leaves the payload as patchable
        // text on the published page.
        let html = strip_active("<p>a</p><script>alert(1)</script><p>b</p>");
        assert!(!html.contains("alert(1)"), "{html}");
        assert!(html.contains("<p>a</p><p>b</p>"), "{html}");
        // An embed that closes nowhere still drops only itself.
        let cut = strip_active("<p>a</p><iframe src=\"x\"><p>b</p>");
        assert!(!cut.contains("<iframe"), "{cut}");
        assert!(cut.contains("<p>b</p>"), "{cut}");
    }

    #[test]
    fn an_uppercased_script_tag_is_dropped_too() {
        // The renderer passes the note's raw HTML through, and a browser
        // runs <SCRIPT> exactly like <script>.
        let html = strip_active("<p>keep</p><SCRIPT>alert(1)</SCRIPT><p>tail</p>");
        assert!(!html.to_lowercase().contains("<script"), "{html}");
        assert!(html.contains("keep"), "{html}");
    }

    #[test]
    fn a_split_script_tag_cannot_reassemble_into_a_live_one() {
        // <scr<script>x</script>ipt> leaves <scr + ipt> = a whole <script>
        // in the output when the pass runs only once.
        let html = strip_active("<scr<script>x</script>ipt>alert(1)</script>");
        assert!(!html.to_lowercase().contains("<script"), "{html}");
        assert!(!html.contains("alert(1)"), "{html}");
    }

    #[test]
    fn a_render_drops_active_content_but_keeps_the_note_around_it() {
        let html = strip_active(
            "<p>before<script>alert(1)</script>after<iframe src=\"x\"></iframe>end</p>",
        );
        assert!(!html.contains("<script"), "{html}");
        assert!(!html.contains("<iframe"), "{html}");
        assert!(html.contains("before"), "{html}");
        assert!(html.contains("after"), "{html}");
        assert!(html.contains("end"), "{html}");
        // A truncated tag drops itself, not the rest of the page.
        let cut = strip_active("<p>keep</p><script>never closed");
        assert!(cut.contains("<p>keep</p>"), "{cut}");
        assert!(!cut.contains("<script"), "{cut}");
    }

    #[test]
    fn the_export_skips_notes_in_the_vaults_own_directories() {
        let files = [
            note("Note.md"),
            note(".trash/Old.md"),
            note(".obsidian/config.md"),
            note("Sub/.git/x.md"),
            note("Sub/Deep/Plan.md"),
        ];
        let kept: Vec<&str> = publishable_notes(&files)
            .iter()
            .map(|f| f.path.as_str())
            .collect();
        assert_eq!(kept, ["Note.md", "Sub/Deep/Plan.md"]);
    }

    #[test]
    fn the_output_folder_may_neither_live_in_the_vault_nor_hold_other_files() {
        let root = std::env::temp_dir().join("silica-site-test-root");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("a root");
        let inside = root.join("_site");
        std::fs::create_dir_all(&inside).expect("a folder in the vault");
        assert!(output_is_safe(&inside, &root).is_err());
        assert!(output_is_safe(&root, &root).is_err());

        let fresh = std::env::temp_dir().join("silica-site-test-fresh");
        let _ = std::fs::remove_dir_all(&fresh);
        assert!(output_is_safe(&fresh, &root).is_ok());

        let foreign = std::env::temp_dir().join("silica-site-test-mixed");
        let _ = std::fs::remove_dir_all(&foreign);
        std::fs::create_dir_all(&foreign).expect("a folder");
        std::fs::write(foreign.join("thesis.docx"), "x").expect("a foreign file");
        assert!(output_is_safe(&foreign, &root).is_err());

        // A previous pass's own output is accepted.
        std::fs::remove_file(foreign.join("thesis.docx")).expect("clean it");
        std::fs::write(foreign.join("index.html"), "x").expect("a page");
        assert!(output_is_safe(&foreign, &root).is_ok());
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&fresh);
        let _ = std::fs::remove_dir_all(&foreign);
    }

    #[test]
    fn an_event_handler_attribute_does_not_survive() {
        let html = strip_active("<p>a</p><img src=\"x\" onerror=\"alert(1)\"><p>b</p>");
        assert!(!html.contains("onerror"), "onerror survived: {html}");
        assert!(html.contains("<img src=\"x\""), "the img was lost: {html}");
    }

    #[test]
    fn a_javascript_url_does_not_survive() {
        let html = strip_active("<a href=\"javascript:alert(1)\">x</a>");
        assert!(
            !html.to_ascii_lowercase().contains("javascript:"),
            "javascript: survived: {html}"
        );
        assert!(html.contains(">x</a>"), "the link was lost: {html}");
    }

    #[test]
    fn a_cased_and_unquoted_handler_does_not_survive() {
        let html = strip_active("<IMG ONERROR=alert(1) src=x>");
        assert!(
            !html.to_ascii_lowercase().contains("onerror"),
            "ONERROR survived: {html}"
        );
        assert!(html.contains("src=x"), "src was lost: {html}");
    }

    #[test]
    fn ordinary_links_and_images_are_untouched() {
        let src = "<a href=\"https://example.com/a?b=1&amp;c=2\">go</a>\
<img src=\"pic.png\" alt=\"a>b\"><img src=\"data:image/png;base64,AAAA\">";
        assert_eq!(strip_active(src), src);
    }

    #[test]
    fn a_gt_inside_a_value_does_not_end_the_tag() {
        let html = strip_active("<a title=\"a>b\" href=\"javascript:alert(1)\">x</a>");
        assert!(
            !html.to_ascii_lowercase().contains("javascript:"),
            "javascript: survived: {html}"
        );
        assert!(html.contains("title=\"a>b\""), "title was lost: {html}");
    }

    #[test]
    fn a_data_document_url_does_not_survive() {
        let html = strip_active("<object data=\"data:text/html,<b>\"></object>");
        assert!(
            !html.to_ascii_lowercase().contains("data:text/html"),
            "data:text/html survived: {html}"
        );
    }
}
