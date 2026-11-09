//! The attachment toolbar behind the editor: the vault path an outside
//! file is stored at, and the embed that puts it in the note.
//!
//! Every write goes through [`crate::workspace::VaultState::save_attachment`],
//! which resolves its target through `Vault::write_path` — so a path that
//! would escape the vault is refused there as well. The rules live here as
//! pure functions over strings and one "does this path exist?" closure, so
//! they are tested without a vault, a window or a filesystem: the toolbar
//! only adds the desktop's dialog and the buffer edit around them.
//!
//! Three of them earn their keep:
//!
//! * A picked file keeps only its own name. A dialog hands back whole
//!   paths; storing one verbatim would either fail in `write_path` or,
//!   worse, land the file somewhere the note did not ask for.
//! * A name already taken gets ` 1`, ` 2`, ... appended, the way every
//!   vault app numbers a taken name, so attaching the same picture twice
//!   keeps both copies.
//! * An absolute path, a `..` component or a trailing dot or space is
//!   refused up front rather than rewritten: Windows silently drops the
//!   dot or space from a path component, so `photo.png.` would land on a
//!   different file than the one named.

use std::ops::Range;
use std::path::Path;

use gpui::{div, prelude::*, px, Context, IntoElement, ParentElement, Styled};
use silica_suggest::Options;

use crate::editor::Editor;
use crate::theme::Theme;

/// Extensions the "Insert image" row accepts. Everything else is a plain
/// attachment, which the same toolbar stores but embeds as a file.
const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "tiff", "tif", "avif", "ico",
];

/// How many numbered candidates a name is tried at before giving up. The
/// vault would keep going; a folder holding ten thousand files named the
/// same is a mistake, not a collision.
const MAX_NUMBERED: usize = 10_000;

/// Why a name cannot be stored where it was going to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameRefusal {
    /// No name at all: an outside path with no file name in it.
    Empty,
    /// A path component Windows would rewrite: `photo.png.` or `photo `,
    /// which reach a different file than the one spelled.
    Trailing(String),
    /// A path that reaches outside the vault: absolute, or through `..`.
    Escapes(String),
    /// Every numbered candidate for the name is taken.
    Taken(String),
}

impl NameRefusal {
    /// The sentence the status bar shows for this refusal.
    pub fn message(&self) -> String {
        match self {
            NameRefusal::Empty => "That file has no name".to_string(),
            NameRefusal::Trailing(name) => {
                format!("A name cannot end with a dot or a space: {name}")
            }
            NameRefusal::Escapes(path) => {
                format!("A file cannot be stored outside the vault: {path}")
            }
            NameRefusal::Taken(name) => {
                format!("Every free name for {name} is taken")
            }
        }
    }
}

/// Whether `rel` is a vault-relative path a write may go to: nothing
/// absolute, no `..`, and no component Windows would rewrite. `.` and a
/// doubled separator are harmless and collapse on their own.
pub fn check_relative(rel: &str) -> Result<(), NameRefusal> {
    if rel.trim().is_empty() {
        return Err(NameRefusal::Empty);
    }
    // * A backslash is a separator to Windows and a filename character
    //   everywhere else, so what one caller spells `Sub\a.png` another
    //   spells `Sub/a.png`: read them the same way before refusing either.
    let normalized = rel.replace('\\', "/");
    if normalized.starts_with('/') || is_windows_absolute(&normalized) {
        return Err(NameRefusal::Escapes(rel.to_string()));
    }
    for part in normalized.split('/') {
        match part {
            "" | "." => {}
            ".." => return Err(NameRefusal::Escapes(rel.to_string())),
            part if part.ends_with('.') || part.ends_with(' ') => {
                return Err(NameRefusal::Trailing(part.to_string()))
            }
            _ => {}
        }
    }
    Ok(())
}

/// `C:something` and `\\server\share` both reach outside the vault.
fn is_windows_absolute(path: &str) -> bool {
    let drive =
        path.len() >= 2 && path.as_bytes()[1] == b':' && path.as_bytes()[0].is_ascii_alphabetic();
    drive || path.starts_with("//")
}

/// The name an outside file is stored under: its own file name, with the
/// folders it was picked from thrown away.
pub fn stored_name(source: &Path) -> Result<String, NameRefusal> {
    let Some(name) = source.file_name().and_then(|n| n.to_str()) else {
        return Err(NameRefusal::Empty);
    };
    check_relative(name)?;
    Ok(name.replace('\\', "/"))
}

/// `("archive", "tar.gz")`: the split is at the last dot, and a name
/// whose only dot leads (`.gitignore`) has no extension.
pub fn split_extension(name: &str) -> (&str, &str) {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, ext),
        _ => (name, ""),
    }
}

/// `folder` and `name` joined by `/`, the vault's own spelling.
fn join_path(folder: &str, name: &str) -> String {
    if folder.is_empty() {
        name.to_string()
    } else {
        format!("{folder}/{name}")
    }
}

/// The first free vault-relative path for `name` in `folder`: the plain
/// path when nothing is at it, otherwise `stem 1.ext`, `stem 2.ext`, ...
///
/// `exists` is asked about each candidate in turn — the vault's own
/// index for a real attach, a set of names inside one dialog's picks.
pub fn first_free(
    folder: &str,
    name: &str,
    exists: impl Fn(&str) -> bool,
) -> Result<String, NameRefusal> {
    let (stem, ext) = split_extension(name);
    let candidate = |suffix: &str| {
        let named = if ext.is_empty() {
            suffix.to_string()
        } else {
            format!("{suffix}.{ext}")
        };
        join_path(folder, &named)
    };
    let plain = candidate(stem);
    check_relative(&plain)?;
    if !exists(&plain) {
        return Ok(plain);
    }
    for n in 1..=MAX_NUMBERED {
        let candidate = candidate(&format!("{stem} {n}"));
        check_relative(&candidate)?;
        if !exists(&candidate) {
            return Ok(candidate);
        }
    }
    Err(NameRefusal::Taken(name.to_string()))
}

/// Whether "Insert image" accepts this name. A picture keeps its own
/// extension; a file with none at all is a plain attachment.
pub fn is_image_name(name: &str) -> bool {
    let (_, ext) = split_extension(name);
    IMAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str())
}

/// The embed line that puts the file written as `written` in the note.
pub fn embed(opts: &Options, written: &str) -> String {
    if opts.markdown_links {
        // * Only the spaces are encoded in a markdown destination.
        format!("![]({})", written.replace(' ', "%20"))
    } else {
        format!("![[{written}]]")
    }
}

/// One embed in a note: `![[a/b.png]]` or `![alt](a/b.png)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Embed {
    /// The whole embed, from its `!` to the closing `]]` or `)`.
    pub span: Range<usize>,
    /// The bracket body, or the destination (and title) of a markdown link.
    pub inner: String,
    /// Which of the two shapes it is.
    pub wikilink: bool,
}

/// Every embed in `text`, in order. An unterminated one is not an embed:
/// half of `![[a/b.png` is typed text.
pub fn embeds(text: &str) -> Vec<Embed> {
    let mut out = Vec::new();
    let mut ix = 0;
    while ix < text.len() {
        if !text[ix..].starts_with("![") {
            ix += 1;
            continue;
        }
        let (open, close, wikilink) = if text[ix..].starts_with("![[") {
            ("[[", "]]", true)
        } else {
            ("[", ")", false)
        };
        let body = ix + 1 + open.len();
        match text[body..].find(close) {
            Some(offset) => {
                let end = body + offset + close.len();
                out.push(Embed {
                    span: ix..end,
                    inner: text[body..body + offset].to_string(),
                    wikilink,
                });
                ix = end;
            }
            None => ix += 1,
        }
    }
    out
}

/// The embed `at` sits on: the one whose span holds it, or the first one
/// on its line that does not end before it. `None` when the cursor sits
/// past the last embed on its line, or on a line with none at all.
pub fn attachment_at(text: &str, at: usize) -> Option<Embed> {
    let at = at.min(text.len());
    let found = embeds(text);
    if let Some(embed) = found.iter().find(|e| e.span.contains(&at)) {
        return Some(embed.clone());
    }
    let line = line_range(text, at);
    found.into_iter().find(|e| {
        e.span.end >= at
            && e.span.start >= line.start
            && e.span.start < line.end
            && e.span.end <= line.end
    })
}

/// The bytes the embed occupies: the whole line when the embed is alone on
/// it, the embed and the spaces after it otherwise. Removing just the
/// span would leave a blank line, or a lone `!` and its brackets.
pub fn removal_range(text: &str, span: Range<usize>) -> Range<usize> {
    let line = line_range(text, span.start);
    let alone = text[line.clone()].trim() == text[span.clone()].trim();
    if alone {
        let mut end = line.end;
        if text[end..].starts_with("\r\n") {
            end += 2;
        } else if text[end..].starts_with('\n') {
            end += 1;
        }
        return line.start..end;
    }
    let mut end = span.end;
    while end < line.end && text[end..].starts_with(' ') {
        end += 1;
    }
    span.start..end
}

/// The link an embed points at: no alias, no subpath, and the percent
/// encoding of a markdown destination undone. `None` for an embed of the
/// note itself or of a heading in it, which has no file behind it.
pub fn embed_target(embed: &Embed) -> Option<String> {
    let inner = embed.inner.trim();
    let target = if embed.wikilink {
        // * `a/b.png#thumb|A picture` names the file `a/b.png`.
        inner.split(['#', '|']).next().unwrap_or(inner)
    } else {
        // * `a/b.png "A picture"`, or `<a/b.png>` when the destination
        //   holds a space it did not encode.
        let dest = if let Some(angled) = inner.strip_prefix('<') {
            angled.split('>').next().unwrap_or(angled)
        } else {
            // * `![label](a/b.png "A picture")`: the label sits in front of
            //   the destination, between `![` and `](`, and the inner of a
            //   markdown embed holds both. A destination that holds a space
            //   it did not encode cannot be told from its title here, which
            //   is why the toolbar percent-encodes what it writes.
            let after_label = inner.split_once("](").map_or(inner, |(_, dest)| dest);
            after_label.split_whitespace().next().unwrap_or(after_label)
        };
        return (!dest.is_empty()).then(|| percent_decode(dest));
    };
    (!target.is_empty()).then(|| target.to_string())
}

/// Undoes the one encoding a markdown destination carries.
fn percent_decode(dest: &str) -> String {
    let bytes = dest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut ix = 0;
    while ix < bytes.len() {
        if bytes[ix] == b'%' && ix + 2 < bytes.len() {
            let hex = &dest[ix + 1..ix + 3];
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                ix += 3;
                continue;
            }
        }
        out.push(bytes[ix]);
        ix += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| dest.to_string())
}

/// The line `at` is on, `start..end` with the newline left out.
pub fn line_range(text: &str, at: usize) -> Range<usize> {
    let at = at.min(text.len());
    let start = text[..at].rfind('\n').map(|ix| ix + 1).unwrap_or(0);
    let end = text[start..]
        .find('\n')
        .map(|ix| start + ix)
        .unwrap_or(text.len());
    start..end
}

impl Editor {
    /// The attachment toolbar above the text: the two inserts and, when the
    /// cursor sits on an embed, the removal for that embed.
    ///
    /// The removal row asks the same question the button asks — the embed
    /// under the cursor — so what the row can do is decided in one place.
    /// With the cursor off every embed it is drawn grey and inert rather
    /// than hidden: a toolbar whose rows come and go moves the rest of the
    /// row with them.
    pub(crate) fn render_attach_toolbar(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let removable = self.attachment_under_cursor().is_some();
        let row = |id: &'static str, label: &'static str, theme: &Theme| {
            div()
                .id(id)
                .px_2()
                .py_px()
                .rounded_sm()
                .text_size(px(12.))
                .text_color(theme.text)
                .bg(theme.bg_alt)
                .child(label)
        };
        let insert = row("attach-image", "Insert image", theme)
            .cursor_pointer()
            .hover(|s| s.bg(theme.bg_active))
            .on_click(cx.listener(|this, _, window, cx| this.attach_image(window, cx)));
        let attach = row("attach-file", "Attach file", theme)
            .cursor_pointer()
            .hover(|s| s.bg(theme.bg_active))
            .on_click(cx.listener(|this, _, window, cx| this.attach_file(window, cx)));
        let mut remove = row("attach-remove", "Remove", theme);
        remove = if removable {
            remove
                .cursor_pointer()
                .hover(|s| s.bg(theme.bg_active))
                .on_click(cx.listener(|this, _, window, cx| this.remove_attachment(window, cx)))
        } else {
            remove.text_color(theme.text_muted)
        };
        div()
            .id("attach-toolbar")
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .px_2()
            .py_1()
            .bg(theme.bg_secondary)
            .border_b_1()
            .border_color(theme.border)
            .font_family(theme.ui_font.clone())
            .child(insert)
            .child(attach)
            .child(remove)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn free_of<'a>(taken: &'a [&str]) -> impl Fn(&str) -> bool + 'a {
        move |rel| taken.contains(&rel)
    }

    // ----- names and paths -------------------------------------------------

    #[test]
    fn a_picked_file_keeps_only_its_own_name() {
        assert_eq!(
            stored_name(Path::new("C:/Users/someone/Desktop/photo.png")),
            Ok("photo.png".to_string())
        );
        assert_eq!(
            stored_name(Path::new("../notes/Field card.md")),
            Ok("Field card.md".to_string())
        );
    }

    #[test]
    fn a_name_windows_would_rewrite_is_refused() {
        assert_eq!(
            stored_name(Path::new("photo.png.")),
            Err(NameRefusal::Trailing("photo.png.".to_string()))
        );
        assert_eq!(
            check_relative("Sub/notes /plan.md"),
            Err(NameRefusal::Trailing("notes ".to_string()))
        );
    }

    #[test]
    fn a_path_outside_the_vault_is_refused() {
        for refused in [
            "/etc/passwd",
            "C:/Windows/x",
            "\\\\server\\share\\x",
            "../escape.png",
            "Sub/../../x.png",
        ] {
            assert_eq!(
                check_relative(refused).err(),
                Some(NameRefusal::Escapes(refused.to_string())),
                "{refused} must not pass"
            );
        }
    }

    #[test]
    fn a_dot_and_a_doubled_separator_collapse() {
        assert_eq!(check_relative("./Sub//a.png"), Ok(()));
        assert_eq!(check_relative("Sub/a.png"), Ok(()));
        assert_eq!(check_relative("   ").err(), Some(NameRefusal::Empty));
    }

    #[test]
    fn a_free_name_is_used_verbatim_in_its_folder() {
        let taken: [&str; 0] = [];
        assert_eq!(
            first_free("Attachments", "photo.png", free_of(&taken)),
            Ok("Attachments/photo.png".to_string())
        );
        assert_eq!(
            first_free("", "photo.png", free_of(&taken)),
            Ok("photo.png".to_string())
        );
    }

    #[test]
    fn a_taken_name_gets_numbered_from_one() {
        let taken = ["Attachments/photo.png", "Attachments/photo 1.png"];
        assert_eq!(
            first_free("Attachments", "photo.png", free_of(&taken)),
            Ok("Attachments/photo 2.png".to_string())
        );
    }

    #[test]
    fn an_extensionless_name_is_numbered_without_a_dangling_dot() {
        let taken = ["LICENSE", "LICENSE 1", "LICENSE 2"];
        assert_eq!(
            first_free("", "LICENSE", free_of(&taken)),
            Ok("LICENSE 3".to_string())
        );
    }

    #[test]
    fn a_folder_full_of_the_same_name_gives_up_rather_than_looping() {
        // * One closure that says yes to everything: the only thing that
        //   ends this is the cap.
        assert_eq!(
            first_free("", "photo.png", |_| true),
            Err(NameRefusal::Taken("photo.png".to_string()))
        );
    }

    #[test]
    fn a_name_that_cannot_be_stored_anywhere_is_refused_before_it_is_tried() {
        assert_eq!(
            first_free("/etc", "photo.png", |_| false).err(),
            Some(NameRefusal::Escapes("/etc/photo.png".to_string()))
        );
    }

    #[test]
    fn only_pictures_are_images() {
        for image in ["a.png", "a.JPEG", "a.Svg", "a.tar.gz", "noext"] {
            let expected = image != "a.tar.gz" && image != "noext";
            assert_eq!(is_image_name(image), expected, "{image}");
        }
    }

    #[test]
    fn extensions_split_at_the_last_dot() {
        assert_eq!(split_extension("archive.tar.gz"), ("archive.tar", "gz"));
        assert_eq!(split_extension(".gitignore"), (".gitignore", ""));
        assert_eq!(split_extension("LICENSE"), ("LICENSE", ""));
    }

    // ----- embeds ----------------------------------------------------------

    #[test]
    fn the_embed_matches_the_link_settings() {
        let markdown = Options {
            format: silica_cache::links::LinkFormat::Shortest,
            markdown_links: true,
        };
        let wikilink = Options {
            markdown_links: false,
            ..markdown
        };
        assert_eq!(
            embed(&wikilink, "Attachments/photo.png"),
            "![[Attachments/photo.png]]"
        );
        assert_eq!(
            embed(&markdown, "Attachments/my photo.png"),
            "![](Attachments/my%20photo.png)"
        );
    }

    #[test]
    fn both_embed_shapes_are_found_in_order() {
        let text = "a ![[one.png]] b ![alt](two.png) c ![[three.png|Three]]";
        let found = embeds(text);
        assert_eq!(found.len(), 3);
        assert_eq!(&text[found[0].span.clone()], "![[one.png]]");
        assert!(found[0].wikilink);
        assert_eq!(&text[found[1].span.clone()], "![alt](two.png)");
        assert!(!found[1].wikilink);
        assert_eq!(&text[found[2].span.clone()], "![[three.png|Three]]");
    }

    #[test]
    fn an_unterminated_embed_is_not_one() {
        assert!(embeds("![[one.png and ![[more").is_empty());
    }

    #[test]
    fn the_embed_under_the_cursor_is_the_one_found() {
        let text = "one ![[a.png]] two ![[b.png]] three";
        let in_b = text.find("![[b.png]]").unwrap() + 4;
        assert_eq!(
            attachment_at(text, in_b).map(|e| e.inner),
            Some("b.png".to_string())
        );
        // * A cursor in the gap between the two names the second one: the
        //   first that does not already end behind the cursor.
        let gap = text.find(" two ").unwrap() + 1;
        assert_eq!(
            attachment_at(text, gap).map(|e| e.inner),
            Some("b.png".to_string())
        );
        // * Past the last embed on the line, nothing is named.
        assert!(attachment_at(text, 30).is_none());
    }

    #[test]
    fn a_target_drops_its_alias_subpath_and_encoding() {
        let text = "![[Attach/a.png#thumb|A picture]] ![x](Attach/my%20a.png \"t\")";
        let found = embeds(text);
        assert_eq!(embed_target(&found[0]), Some("Attach/a.png".to_string()));
        assert_eq!(embed_target(&found[1]), Some("Attach/my a.png".to_string()));
    }

    #[test]
    fn an_embed_of_the_note_itself_has_no_file() {
        let found = embeds("![[#The lake]]");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].inner, "#The lake");
        assert_eq!(embed_target(&found[0]), None);
    }

    // ----- removal ---------------------------------------------------------

    #[test]
    fn an_embed_alone_on_its_line_takes_the_line_with_it() {
        let text = "before\n![[a.png]]\nafter";
        let span = 7..17;
        assert_eq!(&text[span.clone()], "![[a.png]]");
        let range = removal_range(text, span);
        assert_eq!(&text[range.clone()], "![[a.png]]\n");
        assert_eq!(text.replacen("![[a.png]]\n", "", 1), "before\nafter");
    }

    #[test]
    fn an_embed_sharing_its_line_keeps_the_line() {
        let text = "See ![[a.png]] here";
        let span = 4..14;
        assert_eq!(&text[span.clone()], "![[a.png]]");
        let range = removal_range(text, span);
        assert_eq!(&text[range.clone()], "![[a.png]] ");
        assert_eq!(text.replacen("![[a.png]] ", "", 1), "See here");
    }

    #[test]
    fn line_ranges_leave_the_newline_out() {
        assert_eq!(line_range("a\nbc\n", 3), 2..4);
        assert_eq!(line_range("a\nbc\n", 0), 0..1);
        assert_eq!(line_range("abc", 99), 0..3);
    }

    // ----- the toolbar's rows ----------------------------------------------

    #[test]
    fn the_remove_row_is_offered_only_on_an_embed() {
        // * The row asks the same question the button asks: the embed under
        //   the cursor. On one it can act; between two it names the next.
        let text = "one ![[a.png]] two ![[b.png]] three";
        let on_a = text.find("![[a.png]]").unwrap() + 4;
        assert!(attachment_at(text, on_a).is_some());
        let gap = text.find(" two ").unwrap() + 1;
        assert!(
            attachment_at(text, gap).is_some(),
            "a cursor between two embeds on a line still names one of them"
        );
        assert!(attachment_at("plain text, no embeds", 3).is_none());
        // * An embed of the note itself has no file behind it, so the row
        //   finds an embed and the removal then has nothing to point at.
        let found = embeds("![[#Heading]]");
        assert!(attachment_at("![[#Heading]]", 3).is_some());
        assert_eq!(embed_target(&found[0]), None);
    }
}
