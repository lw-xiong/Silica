//! What the binary carries with it: the icons the shell draws, the logo it
//! signs the window with, and the interface font it falls back to.
//!
//! The icons are Lucide's (ISC, `assets/icons/LICENSE`), fetched from the
//! Lucide repository rather than from any Obsidian bundle; Obsidian names
//! the Lucide icon each control uses in its DOM (`svg.lucide-search`), and
//! that name is what [`icon`] takes. Two of the names Obsidian uses are its
//! own drawings rather than Lucide's — `links-coming-in` and
//! `links-going-out` on the right dock's Backlinks and Outgoing links tabs —
//! so Silica draws its own under the same names: Lucide's chain at the top
//! left with a horizontal arrow beneath it, pointing in or out. The font is Inter (SIL OFL,
//! `assets/fonts/LICENSE-Inter.txt`), the family at the head of Obsidian's
//! own interface and text font stacks, in the static faces the pinned gpui
//! can pick weights from (a variable font renders every weight as regular).
//! The logo is ours, drawn for the project; see `docs/img/` for the master.

use anyhow::Result;
use gpui::{img, px, svg, AssetSource, Hsla, Img, SharedString, Styled, Svg};
use std::borrow::Cow;

/// The asset source handed to gpui: every path `svg().path(..)` and
/// `img(..)` can name.
pub struct Assets;

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        const ICONS: &[(&str, &[u8])] = &[
            $((concat!("icons/", $name, ".svg"), include_bytes!(concat!("../assets/icons/", $name, ".svg")))),*
        ];
    };
}

icons!(
    "align-left",
    "archive",
    "arrow-down",
    "arrow-down-left",
    "arrow-left",
    "arrow-left-right",
    "arrow-right",
    "arrow-up",
    "arrow-up-narrow-wide",
    "arrow-up-right",
    "book-open",
    "bug",
    "binary",
    "bookmark",
    "calendar",
    "case-sensitive",
    "circle-check",
    "circle-help",
    "clipboard",
    "clipboard-list",
    "clock",
    "check",
    "chevron-down",
    "chevron-right",
    "chevron-up",
    "chevrons-down-up",
    "chevrons-up-down",
    "code",
    "code-xml",
    "copy",
    "ellipsis",
    "ellipsis-vertical",
    "file-plus",
    "file-question",
    "file-search",
    "file",
    "file-text",
    "files",
    "flame",
    "flask-conical",
    "folder-closed",
    "folder-open",
    "folder-search",
    "folder-tree",
    "folder-plus",
    "forward",
    "gallery-vertical",
    "git-fork",
    "heading",
    "image",
    "info",
    "layers",
    "kanban-square",
    "layout-dashboard",
    "layout-grid",
    "layout-list",
    "layout-template",
    "link",
    "links-coming-in",
    "links-going-out",
    "list",
    "list-start",
    "maximize",
    "minus",
    "move-horizontal",
    "move-vertical",
    "palette",
    "panel-left",
    "panel-right",
    "pen-line",
    "pencil",
    "pin",
    "plus",
    "quote",
    "rectangle-vertical",
    "redo-2",
    "rotate-ccw",
    "search",
    "separator-vertical",
    "settings",
    "sliders-horizontal",
    "sticky-note",
    "square",
    "square-check",
    "square-pen",
    "table",
    "tags",
    "terminal",
    "text",
    "trash-2",
    "triangle-alert",
    "type",
    "undo-2",
    "x",
    "zap",
    "zoom-in",
    "zoom-out",
);

/// The logo, the one bundled asset that is not a monochrome icon. It is
/// loaded through [`img`] rather than [`svg`] because gpui draws an `svg()`
/// as a single-colour mask and rasterizes an `img()` through resvg, which is
/// what keeps the mark's gradients.
const LOGO: (&str, &[u8]) = ("logo.png", include_bytes!("../assets/logo.png"));

/// The mark's width over its height. Setting both axes from it keeps the
/// aspect ratio off the layout's hands. The new logo is square.
const LOGO_ASPECT: f32 = 1.0;

/// Every path [`Assets`] answers to.
fn bundled() -> impl Iterator<Item = &'static (&'static str, &'static [u8])> {
    ICONS.iter().chain(std::iter::once(&LOGO))
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(bundled()
            .find(|(p, _)| *p == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(bundled()
            .filter(|(p, _)| p.starts_with(path))
            .map(|(p, _)| SharedString::from(*p))
            .collect())
    }
}

/// Inter's static faces, registered with the text system at startup so the
/// family resolves on a machine that does not have it installed.
pub fn fonts() -> Vec<Cow<'static, [u8]>> {
    vec![
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Regular.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Italic.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Medium.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-SemiBold.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Bold.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-BoldItalic.ttf")),
    ]
}

/// The family [`fonts`] registers.
pub const FONT_FAMILY: &str = "Inter";

/// A Lucide icon by name, `size` pixels square, painted in `color`.
/// Obsidian's icons are 16 px with a 2 px stroke on a 24-unit grid, which is
/// Lucide's own drawing, so a 16 px box here is the same picture.
pub fn icon(name: &str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(SharedString::from(format!("icons/{name}.svg")))
        .size(px(size))
        .flex_shrink_0()
        .text_color(color)
}

/// The Lucide icon Obsidian draws in a callout's title row, by canonical
/// callout type (see [`silica_markdown::canonical_callout_type`]). The
/// pairing was read from the rendered `.callout-icon svg`'s own class in
/// Obsidian 1.14.5; the table is in `docs/silica/Specification.md` under
/// "The lucide icon and colour of every built-in callout type". Two of the
/// names Obsidian leaves there are Lucide's older ones, and the drawing is
/// bundled under its current name instead: `check-circle-2` is
/// `circle-check`, `alert-triangle` is `triangle-alert`. A type Obsidian
/// does not know keeps its own name in the DOM but takes `note`'s pencil,
/// which it already has by the time it arrives here — the canonical type of
/// anything unrecognized is `note`.
pub fn callout_icon(kind: &str) -> &'static str {
    match kind {
        "abstract" => "clipboard-list",
        "info" => "info",
        "todo" => "circle-check",
        "tip" => "flame",
        "success" => "check",
        "question" => "circle-help",
        "warning" => "triangle-alert",
        "failure" => "x",
        "danger" => "zap",
        "bug" => "bug",
        "example" => "list",
        "quote" => "quote",
        _ => "pencil",
    }
}

/// The Silica mark, `width` pixels wide and in its own colours.
pub fn logo(width: f32) -> Img {
    img(LOGO.0)
        .w(px(width))
        .h(px(width / LOGO_ASPECT))
        .flex_shrink_0()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bundled_icon_loads_and_is_an_svg() {
        for (path, _) in ICONS {
            let bytes = Assets.load(path).unwrap().expect(path);
            let text = std::str::from_utf8(&bytes).unwrap();
            assert!(text.contains("<svg"), "{path} is not an svg");
            assert!(
                text.contains("viewBox=\"0 0 24 24\""),
                "{path} is not on Lucide's 24-unit grid"
            );
        }
    }

    #[test]
    fn every_callout_type_draws_the_icon_obsidian_draws() {
        // The whole of the probe's table, aliases and all, plus a type
        // Obsidian does not know: it falls back to note's pencil.
        let table = [
            ("note", "pencil"),
            ("abstract", "clipboard-list"),
            ("summary", "clipboard-list"),
            ("tldr", "clipboard-list"),
            ("info", "info"),
            ("todo", "circle-check"),
            ("tip", "flame"),
            ("hint", "flame"),
            ("important", "flame"),
            ("success", "check"),
            ("check", "check"),
            ("done", "check"),
            ("question", "circle-help"),
            ("help", "circle-help"),
            ("faq", "circle-help"),
            ("warning", "triangle-alert"),
            ("caution", "triangle-alert"),
            ("attention", "triangle-alert"),
            ("failure", "x"),
            ("fail", "x"),
            ("missing", "x"),
            ("danger", "zap"),
            ("error", "zap"),
            ("bug", "bug"),
            ("example", "list"),
            ("quote", "quote"),
            ("cite", "quote"),
            ("nosuchtype", "pencil"),
        ];
        for (written, want) in table {
            let name = callout_icon(silica_markdown::canonical_callout_type(written));
            assert_eq!(name, want, "[!{written}]");
            let path = format!("icons/{name}.svg");
            assert!(
                Assets.load(&path).unwrap().is_some(),
                "{path} is not bundled"
            );
        }
        // The fold chevron, drawn in both states.
        for name in ["chevron-down", "chevron-right"] {
            assert!(Assets.load(&format!("icons/{name}.svg")).unwrap().is_some());
        }
    }

    #[test]
    fn the_logo_loads_and_keeps_its_own_colours() {
        let bytes = Assets.load(LOGO.0).unwrap().expect("logo.png");
        // It is a PNG with transparency, no svg markup.
        assert!(
            !std::str::from_utf8(&bytes).unwrap_or("").contains("<svg"),
            "the logo is not an svg"
        );
        // No stop fades out: a mark that ramps to transparent takes the
        // colour of whatever is behind it, which is how the drawing arrived.
        assert!(
            !std::str::from_utf8(&bytes)
                .unwrap_or("")
                .contains("stop-opacity:0;"),
            "the logo has a transparent gradient stop again"
        );
        assert!(Assets.list("").unwrap().iter().any(|p| p == LOGO.0));
    }

    #[test]
    fn listing_filters_by_prefix() {
        assert!(Assets.list("icons/").unwrap().len() == ICONS.len());
        assert!(Assets.list("fonts/").unwrap().is_empty());
        assert!(Assets.load("icons/no-such-icon.svg").unwrap().is_none());
    }

    #[test]
    fn the_fonts_are_truetype() {
        for bytes in fonts() {
            // A TrueType face starts with the 0x00010000 sfnt version.
            assert_eq!(&bytes[..4], &[0, 1, 0, 0]);
        }
    }
}
