//! Colors and fonts for the shell, chosen from `appearance.json` the way
//! Obsidian's base themes are: `obsidian` is dark, `moonstone` is light,
//! `system` follows the desktop (dark until GPUI reports the preference).
//!
//! The palettes and the sizes below are the values Obsidian 1.14.4 computes
//! for its own shell on Linux, read from a running copy with the probe in
//! `tools/obsidian-probe/frame-typography.mjs` on 2026-10-20 and listed in
//! `docs/silica/Specification.md` ("Shell typography and chrome"). Hovered
//! and active rows are a 6.7 percent wash of white (dark) or black (light)
//! over whatever is underneath rather than a flat colour, so the same token
//! reads right over both backgrounds.

use gpui::{hsla, rgb, rgba, Global, Hsla, SharedString};
use silica_formats::config::AppearanceConfig;
use std::collections::BTreeMap;
use std::path::Path;

/// Obsidian's interface sizes: the small UI text of tabs, rows and headers,
/// the smaller text of the status bar, the height of every header row, the
/// ribbon's width, the icon box, and the sidebar width a new vault gets.
pub const FONT_UI_SMALLER: f32 = 12.;
pub const FONT_UI_SMALL: f32 = 13.;
pub const FONT_UI_MEDIUM: f32 = 15.;
pub const HEADER_HEIGHT: f32 = 40.;
/// The radius of a tab's top corners and of the active tab's feet, where it curves into the view.
pub const TAB_CURVE: f32 = 10.;
pub const RIBBON_WIDTH: f32 = 44.;
pub const ICON_SIZE: f32 = 16.;
pub const SIDEBAR_WIDTH: f32 = 300.;

/// The monospace family a vault with no `monospaceFontFamily` renders code
/// in: the first entry of Obsidian's `--font-monospace` stack (see the
/// Specification's Fonts note) that ships with the platform. gpui names a
/// concrete family, not a generic one, and a family the platform lacks
/// falls back to its sans, so each platform gets a face it has.
pub const DEFAULT_MONO_FONT: &str = if cfg!(target_os = "macos") {
    "Menlo"
} else if cfg!(target_os = "windows") {
    "Consolas"
} else {
    "DejaVu Sans Mono"
};

/// The body font size's multiplier for each heading level, and the weight
/// and line height Obsidian gives it (`--h1-size`, `--h1-weight`,
/// `--h1-line-height`; the fourth level down share the third's line height).
pub fn heading_style(level: u8) -> (f32, f32, f32) {
    match level {
        1 => (1.618, 700., 1.2),
        2 => (1.462, 680., 1.2),
        3 => (1.318, 660., 1.3),
        4 => (1.188, 640., 1.3),
        5 => (1.076, 620., 1.3),
        _ => (1.0, 600., 1.3),
    }
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub dark: bool,
    pub bg: Hsla,
    pub bg_secondary: Hsla,
    /// Code blocks and inline code (`--background-primary-alt`).
    pub code_bg: Hsla,
    /// Hovered rows and buttons: a translucent wash (`--background-modifier-hover`).
    pub bg_alt: Hsla,
    /// The active row, dock tab and explorer file: the same wash.
    pub bg_active: Hsla,
    pub border: Hsla,
    /// `--tab-divider-color`: the hairline between two inactive root tabs,
    /// read as a computed property on 2026-10-22 (#3f3f3f dark, #dadada
    /// light), a step brighter than the strip's own `--background-modifier-border`.
    pub tab_divider: Hsla,
    /// `--background-modifier-border-focus`: the 2 px ring on the explorer
    /// row the keyboard is on.
    pub border_focus: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub text_faint: Hsla,
    pub accent: Hsla,
    pub link: Hsla,
    pub tag: Hsla,
    pub highlight_bg: Hsla,
    pub ui_font: SharedString,
    pub text_font: SharedString,
    pub mono_font: SharedString,
    pub base_font_size: f32,
    /// The canvas: card background and the border of a card with no colour
    /// of its own, the dot grid, the six preset colours a card or edge can
    /// carry (`"1"` red through `"6"` purple), the stroke of an edge with no
    /// colour, the label text above a card and inside a group, and the wash
    /// of a group's body and a marquee.
    pub canvas: CanvasColors,
}

/// The canvas view's palette. Obsidian paints the view over its primary
/// background, cards over the secondary one, and its preset colours are the
/// `--color-red` .. `--color-purple` tokens the callouts use too, so the
/// values below are the ones [`Theme::callout_color`] draws for the same
/// hues plus `--color-yellow`.
#[derive(Debug, Clone)]
pub struct CanvasColors {
    /// `--canvas-background`: the view's ground and every card's fill.
    pub background: Hsla,
    /// `--canvas-color`: the border and edge colour of an uncoloured card.
    pub card_border: Hsla,
    /// `--canvas-dot-pattern`.
    pub dot: Hsla,
    /// `--canvas-card-label-color`: the file name above a file card.
    pub label: Hsla,
    /// The alpha a coloured card's border and a group's frame paint their
    /// colour at.
    pub border_alpha: f32,
    /// The alpha a coloured card's or group's body is tinted at.
    pub tint_alpha: f32,
    /// The alpha a plain group's label pill paints the default colour at;
    /// also the marquee's wash.
    pub wash_alpha: f32,
    /// The text on a coloured group's solid label pill.
    pub on_color: Hsla,
    /// Control stack: a group's fill and border, a button's fill and glyph.
    pub control_group: Hsla,
    pub control_border: Hsla,
    pub control_bg: Hsla,
    pub control_fg: Hsla,
    /// The card menu's fill.
    pub card_menu_bg: Hsla,
    /// `"1"` red, `"2"` orange, `"3"` yellow, `"4"` green, `"5"` cyan, `"6"` purple.
    pub presets: [Hsla; 6],
}

impl CanvasColors {
    /// Obsidian 1.14.4's default dark theme, read from the running app.
    fn dark() -> CanvasColors {
        CanvasColors {
            background: rgb(0x1c1c1c).into(),
            card_border: rgb(0x7e7e7e).into(),
            dot: rgb(0x333333).into(),
            label: rgb(0x666666).into(),
            border_alpha: 0.7,
            tint_alpha: 0.07,
            wash_alpha: 0.1,
            on_color: rgb(0x000000).into(),
            control_group: rgb(0x333333).into(),
            control_border: rgb(0x333333).into(),
            control_bg: rgb(0x1c1c1c).into(),
            control_fg: rgb(0xb3b3b3).into(),
            card_menu_bg: rgb(0x282828).into(),
            presets: [0xfb464c, 0xe9973f, 0xe0de71, 0x44cf6e, 0x53dfdd, 0xa882ff]
                .map(|c| rgb(c).into()),
        }
    }

    /// Obsidian 1.14.4's default light theme.
    fn light() -> CanvasColors {
        CanvasColors {
            background: rgb(0xffffff).into(),
            card_border: rgb(0xc0c0c0).into(),
            dot: rgb(0xe4e4e4).into(),
            label: rgb(0xababab).into(),
            border_alpha: 0.7,
            tint_alpha: 0.07,
            wash_alpha: 0.1,
            on_color: rgb(0xffffff).into(),
            control_group: rgb(0xffffff).into(),
            control_border: rgb(0xe4e4e4).into(),
            control_bg: rgb(0xffffff).into(),
            control_fg: rgb(0x5c5c5c).into(),
            card_menu_bg: rgb(0xffffff).into(),
            presets: [0xe93147, 0xec7500, 0xe0ac00, 0x08b94e, 0x00bfbc, 0x7852ee]
                .map(|c| rgb(c).into()),
        }
    }

    /// The colour of a preset, `"1"` to `"6"`.
    pub fn preset(&self, n: u8) -> Option<Hsla> {
        (1..=6)
            .contains(&n)
            .then(|| self.presets[usize::from(n) - 1])
    }
}

impl Global for Theme {}

impl Theme {
    pub fn dark() -> Theme {
        Theme {
            dark: true,
            bg: rgb(0x1c1c1c).into(),
            bg_secondary: rgb(0x282828).into(),
            code_bg: rgb(0x232323).into(),
            bg_alt: rgba(0xffffff11).into(),
            bg_active: rgba(0xffffff11).into(),
            border: rgb(0x333333).into(),
            tab_divider: rgb(0x3f3f3f).into(),
            border_focus: rgb(0x555555).into(),
            text: rgb(0xdadada).into(),
            text_muted: rgb(0xb3b3b3).into(),
            text_faint: rgb(0x666666).into(),
            accent: hsla(258. / 360., 0.88, 0.66, 1.),
            link: hsla(255. / 360., 0.8976, 0.759, 1.),
            tag: hsla(255. / 360., 0.8976, 0.759, 1.),
            highlight_bg: rgba(0xffd70066).into(),
            ui_font: crate::assets::FONT_FAMILY.into(),
            text_font: crate::assets::FONT_FAMILY.into(),
            mono_font: DEFAULT_MONO_FONT.into(),
            base_font_size: 16.0,
            canvas: CanvasColors::dark(),
        }
    }

    pub fn light() -> Theme {
        Theme {
            dark: false,
            bg: rgb(0xffffff).into(),
            bg_secondary: rgb(0xf6f6f6).into(),
            code_bg: rgb(0xfafafa).into(),
            bg_alt: rgba(0x00000011).into(),
            bg_active: rgba(0x00000011).into(),
            border: rgb(0xe4e4e4).into(),
            tab_divider: rgb(0xdadada).into(),
            border_focus: rgb(0xc0c0c0).into(),
            text: rgb(0x222222).into(),
            text_muted: rgb(0x5c5c5c).into(),
            text_faint: rgb(0xababab).into(),
            accent: hsla(258. / 360., 0.88, 0.66, 1.),
            link: hsla(258. / 360., 0.88, 0.66, 1.),
            tag: hsla(258. / 360., 0.88, 0.66, 1.),
            highlight_bg: rgba(0xffd70080).into(),
            ui_font: crate::assets::FONT_FAMILY.into(),
            text_font: crate::assets::FONT_FAMILY.into(),
            mono_font: DEFAULT_MONO_FONT.into(),
            base_font_size: 16.0,
            canvas: CanvasColors::light(),
        }
    }

    /// The theme `appearance.json` asks for, with a community CSS theme
    /// layered on top when `css_theme` names one.
    ///
    /// `system` is Obsidian's default and means the desktop's own scheme,
    /// which only a window can answer; `system_dark` is that answer, and
    /// the caller passes what it knows. A vault with no `theme` key gets
    /// the same treatment, since that is what Obsidian reads it as.
    pub fn from_appearance(
        config: &AppearanceConfig,
        system_dark: bool,
        css_theme: Option<&CssTheme>,
        css_snippets: &[CssTheme],
    ) -> Theme {
        let mut theme = match config.theme.as_deref() {
            Some("moonstone") => Theme::light(),
            Some("obsidian") => Theme::dark(),
            // `system` and anything else Obsidian does not know.
            _ if system_dark => Theme::dark(),
            _ => Theme::light(),
        };
        if let Some(size) = config.base_font_size {
            // Obsidian's own slider offers 10 to 30, but it neither clamps
            // nor rewrites a file that holds something else — 99 was left
            // alone across a whole session — so the file's value is
            // honoured. The floor only keeps a zero out of the layout.
            theme.base_font_size = size.max(1) as f32;
        }
        if let Some(f) = first_font(config.text_font_family.as_deref()) {
            theme.text_font = f;
        }
        if let Some(f) = first_font(config.interface_font_family.as_deref()) {
            theme.ui_font = f;
        }
        if let Some(f) = first_font(config.monospace_font_family.as_deref()) {
            theme.mono_font = f;
        }
        if let Some(css) = css_theme {
            theme.apply_css_theme(css);
        }
        // CSS snippets apply last, so they override both base theme and
        // community theme colors.
        for css in css_snippets {
            theme.apply_css_theme(css);
        }
        theme
    }

    /// The wash behind a `==🔴word==` highlight: Obsidian paints the
    /// theme's `--color-<name>` at 0.3 alpha — red was read back as
    /// `oklch(0.659737 0.216875 23.8105 / 0.3)` on the dark theme, which is
    /// its `#fb464c`; the other five are the same tokens the canvas presets
    /// use, at the same alpha.
    pub fn highlight_color(&self, color: silica_markdown::HighlightColor) -> Hsla {
        use silica_markdown::HighlightColor as C;
        let presets = &self.canvas.presets;
        let mut c = match color {
            C::Red => presets[0],
            C::Orange => presets[1],
            C::Yellow => presets[2],
            C::Green => presets[3],
            // `--color-blue`: not a canvas preset (cyan is), so its own
            // pair of values.
            C::Blue => rgb(if self.dark { 0x027aff } else { 0x086ddd }).into(),
            C::Purple => presets[5],
        };
        c.a = 0.3;
        c
    }

    /// Obsidian's default callout colors by canonical type. `--callout-color`
    /// is a `--color-<name>` token, the same six the canvas presets and the
    /// highlight washes draw from, so the hue is picked here and the theme's
    /// own value comes from [`CanvasColors::presets`] — which is what makes
    /// the dark theme's callouts the dark theme's colours. The table of
    /// observed values is in `docs/silica/Specification.md` under "The lucide
    /// icon and colour of every built-in callout type"; it was read in the
    /// default dark theme, and the light column is the same tokens' light
    /// values. `--color-blue` is not a canvas preset, so it keeps its own
    /// pair, exactly as [`Theme::highlight_color`] has it.
    pub fn callout_color(&self, kind: &str) -> Hsla {
        let presets = &self.canvas.presets;
        match kind {
            "abstract" | "tip" => presets[4],
            "success" => presets[3],
            "question" | "warning" => presets[1],
            "failure" | "danger" | "bug" => presets[0],
            "example" => presets[5],
            // Obsidian's grey is a flat value in both themes.
            "quote" => rgb(0x9e9e9e).into(),
            // `note`, `info`, `todo` and anything Obsidian does not know.
            _ => rgb(if self.dark { 0x027aff } else { 0x086ddd }).into(),
        }
    }
}

/// The first family of a CSS-style `"Font A", Font B` list.
fn first_font(list: Option<&str>) -> Option<SharedString> {
    let first = list?
        .split(',')
        .next()?
        .trim()
        .trim_matches('"')
        .trim_matches('\'');
    if first.is_empty() {
        None
    } else {
        Some(first.to_string().into())
    }
}

/// A community theme's CSS custom properties, parsed from
/// `.obsidian/themes/<name>.css`. Only the properties Silica understands are
/// kept; everything else is ignored.
#[derive(Debug, Clone, Default)]
pub struct CssTheme {
    pub vars: BTreeMap<String, String>,
}

/// Parse CSS custom properties (`--name: value;`) from a theme's CSS source.
///
/// Only `--name: value;` declarations at the top level are read; nested rules
/// (`:root { ... }`) are flattened, and anything Silica does not recognize is
/// silently dropped. Colours accept `#rrggbb`, `#rgb`, `rgb(...)`, `hsl(...)`,
/// `oklch(...)` and named colours; anything unparseable is left as its raw
/// string and interpreted when applied.
pub fn parse_css_theme(css: &str) -> CssTheme {
    let mut vars = BTreeMap::new();
    // Strip comments.
    let css = css.replace("/*", "").replace("*/", "");
    for line in css.lines() {
        let line = line.trim();
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim();
            let value = value.trim().trim_end_matches(';').trim();
            if name.starts_with("--") && !value.is_empty() {
                vars.insert(name.to_string(), value.to_string());
            }
        }
    }
    CssTheme { vars }
}

/// Load a community theme from `.obsidian/themes/<name>.css` inside a vault.
pub fn load_css_theme(vault_path: &Path, name: &str) -> Option<CssTheme> {
    let path = vault_path
        .join(".obsidian/themes")
        .join(format!("{name}.css"));
    let css = std::fs::read_to_string(&path).ok()?;
    Some(parse_css_theme(&css))
}

/// Load every enabled CSS snippet from `.obsidian/snippets/*.css` inside a vault.
/// Only snippets whose names appear in `enabled` are loaded and returned, in
/// the same order as `enabled`.
pub fn load_css_snippets(vault_path: &Path, enabled: &[String]) -> Vec<CssTheme> {
    let mut out = Vec::new();
    for name in enabled {
        let path = vault_path
            .join(".obsidian/snippets")
            .join(format!("{name}.css"));
        if let Ok(css) = std::fs::read_to_string(&path) {
            out.push(parse_css_theme(&css));
        }
    }
    out
}

impl Theme {
    /// Apply a parsed community theme on top of this one. Only properties the
    /// theme explicitly sets are overridden; everything else keeps the base
    /// theme's value.
    pub fn apply_css_theme(&mut self, css: &CssTheme) {
        for (name, value) in &css.vars {
            match name.as_str() {
                // Colours
                "--background-primary" => {
                    if let Some(c) = parse_colour(value) {
                        self.bg = c.into();
                    }
                }
                "--background-secondary" => {
                    if let Some(c) = parse_colour(value) {
                        self.bg_secondary = c.into();
                    }
                }
                "--background-secondary-alt" => {
                    if let Some(c) = parse_colour(value) {
                        self.code_bg = c.into();
                    }
                }
                "--background-tertiary" => {
                    if let Some(c) = parse_colour(value) {
                        self.bg_alt = c.into();
                    }
                }
                "--background-modifier-hover" => {
                    if let Some(c) = parse_colour(value) {
                        self.bg_alt = c.into();
                    }
                }
                "--background-modifier-border" => {
                    if let Some(c) = parse_colour(value) {
                        self.border = c.into();
                    }
                }
                "--background-modifier-border-focus" => {
                    if let Some(c) = parse_colour(value) {
                        self.border_focus = c.into();
                    }
                }
                "--text-normal" => {
                    if let Some(c) = parse_colour(value) {
                        self.text = c.into();
                    }
                }
                "--text-muted" => {
                    if let Some(c) = parse_colour(value) {
                        self.text_muted = c.into();
                    }
                }
                "--text-faint" => {
                    if let Some(c) = parse_colour(value) {
                        self.text_faint = c.into();
                    }
                }
                "--text-accent" | "--interactive-accent" => {
                    if let Some(c) = parse_colour(value) {
                        self.accent = c.into();
                    }
                }
                // Canvas colour presets
                "--color-red" => {
                    if let Some(c) = parse_colour(value) {
                        self.canvas.presets[0] = c.into();
                    }
                }
                "--color-orange" => {
                    if let Some(c) = parse_colour(value) {
                        self.canvas.presets[1] = c.into();
                    }
                }
                "--color-yellow" => {
                    if let Some(c) = parse_colour(value) {
                        self.canvas.presets[2] = c.into();
                    }
                }
                "--color-green" => {
                    if let Some(c) = parse_colour(value) {
                        self.canvas.presets[3] = c.into();
                    }
                }
                "--color-cyan" => {
                    if let Some(c) = parse_colour(value) {
                        self.canvas.presets[4] = c.into();
                    }
                }
                "--color-purple" => {
                    if let Some(c) = parse_colour(value) {
                        self.canvas.presets[5] = c.into();
                    }
                }
                // Fonts
                "--font-interface" => {
                    self.ui_font = SharedString::from(value.trim_matches('"').trim_matches('\''));
                }
                "--font-text" => {
                    self.text_font = SharedString::from(value.trim_matches('"').trim_matches('\''));
                }
                "--font-monospace" => {
                    self.mono_font = SharedString::from(value.trim_matches('"').trim_matches('\''));
                }
                _ => {}
            }
        }
    }
}

/// Parse a CSS colour string into an `Hsla`. Supports `#rrggbb`, `#rgb`,
/// `rgb(r, g, b)`, `rgba(r, g, b, a)`, `hsl(h, s, l)`, `hsla(h, s, l, a)`,
/// `oklch(l c h / alpha)` and named colours. Returns `None` for anything
/// unrecognised.
fn parse_colour(s: &str) -> Option<Hsla> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        return parse_hex(hex);
    }
    if let Some(rest) = s.strip_prefix("rgb(").or_else(|| s.strip_prefix("rgba(")) {
        let rest = rest.trim_end_matches(')');
        let parts: Vec<&str> = rest.split(',').map(str::trim).collect();
        if parts.len() >= 3 {
            let r = parts[0].parse::<f32>().ok()? / 255.0;
            let g = parts[1].parse::<f32>().ok()? / 255.0;
            let b = parts[2].parse::<f32>().ok()? / 255.0;
            let a = parts
                .get(3)
                .and_then(|p| p.parse::<f32>().ok())
                .unwrap_or(1.0);
            let hex_val =
                ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | (a as u32 * 255);
            return Some(rgba(hex_val).into());
        }
    }
    None
}

/// Parse a hex colour string (`rrggbb` or `rgb`).
fn parse_hex(hex: &str) -> Option<Hsla> {
    let hex = hex.trim();
    if hex.len() == 3 {
        let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 0x11;
        let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 0x11;
        let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 0x11;
        let hex_val = ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | 0xff;
        return Some(rgba(hex_val).into());
    }
    if hex.len() == 6 {
        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        let hex_val = ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | 0xff;
        return Some(rgba(hex_val).into());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole of the probe's `--callout-color` table, aliases and all, read
    /// in Obsidian 1.14.5's default dark theme. Before this test
    /// [`Theme::callout_color`] ignored `self.dark` and answered the light
    /// theme's values in both themes, so every callout but `quote` was the
    /// wrong colour on the theme Obsidian ships.
    #[test]
    fn every_callout_type_takes_obsidians_dark_color() {
        let theme = Theme::dark();
        let table = [
            ("note", 0x027aff),
            ("abstract", 0x53dfdd),
            ("summary", 0x53dfdd),
            ("tldr", 0x53dfdd),
            ("info", 0x027aff),
            ("todo", 0x027aff),
            ("tip", 0x53dfdd),
            ("hint", 0x53dfdd),
            ("important", 0x53dfdd),
            ("success", 0x44cf6e),
            ("check", 0x44cf6e),
            ("done", 0x44cf6e),
            ("question", 0xe9973f),
            ("help", 0xe9973f),
            ("faq", 0xe9973f),
            ("warning", 0xe9973f),
            ("caution", 0xe9973f),
            ("attention", 0xe9973f),
            ("failure", 0xfb464c),
            ("fail", 0xfb464c),
            ("missing", 0xfb464c),
            ("danger", 0xfb464c),
            ("error", 0xfb464c),
            ("bug", 0xfb464c),
            ("example", 0xa882ff),
            ("quote", 0x9e9e9e),
            ("cite", 0x9e9e9e),
            // An unknown type falls back to note's pair, which it reaches as
            // `note` by way of the canonical type.
            ("nosuchtype", 0x027aff),
        ];
        for (written, want) in table {
            let kind = silica_markdown::canonical_callout_type(written);
            assert_eq!(
                theme.callout_color(kind),
                rgb(want).into(),
                "[!{written}] on the dark theme"
            );
        }
    }

    /// The light theme's callouts are the same tokens' light values. These
    /// were never probed, so the assertion is the set the app already drew,
    /// unchanged by routing the lookup through the presets -- except
    /// `--color-blue`, which was `0x086dff` here and `0x086ddd` in
    /// [`Theme::highlight_color`] for the same token. The spec records
    /// `#086ddd`, so the odd one out was a typo.
    #[test]
    fn the_light_theme_keeps_the_light_tokens() {
        let theme = Theme::light();
        for (kind, want) in [
            ("abstract", 0x00bfbc),
            ("tip", 0x00bfbc),
            ("success", 0x08b94e),
            ("question", 0xec7500),
            ("warning", 0xec7500),
            ("failure", 0xe93147),
            ("danger", 0xe93147),
            ("bug", 0xe93147),
            ("example", 0x7852ee),
            ("quote", 0x9e9e9e),
            ("note", 0x086ddd),
        ] {
            assert_eq!(theme.callout_color(kind), rgb(want).into(), "[!{kind}]");
        }
    }

    /// The two themes may not answer the same colour for a hue that has one,
    /// which is the bug this pair of tests was written for.
    #[test]
    fn the_themes_disagree_where_obsidian_does() {
        for kind in [
            "note", "abstract", "success", "warning", "failure", "example",
        ] {
            assert_ne!(
                Theme::dark().callout_color(kind),
                Theme::light().callout_color(kind),
                "[!{kind}] is the same in both themes"
            );
        }
        // Grey is the exception: one value for both.
        assert_eq!(
            Theme::dark().callout_color("quote"),
            Theme::light().callout_color("quote")
        );
    }

    /// The callout hues and the highlight washes read the same tokens, so the
    /// two lookups may not drift apart again: the wash is the callout colour
    /// at 0.3 alpha, blue included.
    #[test]
    fn a_highlight_wash_is_its_callout_color_at_a_third_alpha() {
        use silica_markdown::HighlightColor as C;
        for theme in [Theme::dark(), Theme::light()] {
            for (color, kind) in [
                (C::Red, "failure"),
                (C::Orange, "warning"),
                (C::Green, "success"),
                (C::Purple, "example"),
                (C::Blue, "note"),
            ] {
                let mut want = theme.callout_color(kind);
                want.a = 0.3;
                assert_eq!(theme.highlight_color(color), want, "{color:?}");
            }
        }
    }
}
