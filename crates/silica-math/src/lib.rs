//! TeX math typeset into a drawing list.
//!
//! Obsidian hands `$x$` and `$$x$$` to MathJax. Silica hands them to RaTeX,
//! a KaTeX-compatible engine in Rust, and gets back glyphs, rules and
//! paths with absolute positions in em units; the app paints that list with
//! gpui at whatever size the surrounding text has. The fonts are KaTeX's
//! (SIL OFL, `fonts()`), because the engine's metrics are theirs.
//!
//! Typesetting is a pure function of the source and the style, so every
//! result is cached here and the views ask for the same formula once per
//! frame without paying for it. A formula that fails to parse is cached
//! too, with its message.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use ratex_layout::{layout, to_display_list, LayoutOptions};
use ratex_types::{DisplayItem, MathStyle};

pub use ratex_types::PathCommand;

/// A typeset formula. Coordinates are in em: multiply by the font size of
/// the surrounding text to get pixels. The origin is the top left corner of
/// the box; the baseline is at `y = height`.
#[derive(Debug, Clone, PartialEq)]
pub struct Formula {
    pub width: f64,
    /// Above the baseline.
    pub height: f64,
    /// Below the baseline.
    pub depth: f64,
    pub items: Vec<Item>,
}

impl Formula {
    /// Height and depth together.
    pub fn total_height(&self) -> f64 {
        self.height + self.depth
    }
}

/// One drawing instruction. A colour of `None` is the text colour: the
/// engine sets everything black unless the formula says otherwise
/// (`\color{red}{x}`), and black is what a theme repaints.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// A character of one of the KaTeX faces, its baseline origin at
    /// `(x, y)` and its font size `size` em.
    Glyph {
        x: f64,
        y: f64,
        size: f64,
        face: Face,
        ch: char,
        color: Option<Rgba>,
    },
    /// A horizontal rule: fraction bars, overlines, `\hline`.
    Line {
        x: f64,
        y: f64,
        width: f64,
        thickness: f64,
        dashed: bool,
        color: Option<Rgba>,
    },
    /// A filled rectangle (`\colorbox`, `\rule`).
    Rect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        color: Option<Rgba>,
    },
    /// A path in em units relative to `(x, y)`: radical signs, stretched
    /// delimiters, wide accents, cancel strokes. Stroked `STROKE_WIDTH` em
    /// wide when not filled.
    Path {
        x: f64,
        y: f64,
        commands: Vec<PathCommand>,
        fill: bool,
        color: Option<Rgba>,
    },
}

/// The stroke of an unfilled [`Item::Path`], in em.
pub const STROKE_WIDTH: f64 = 0.0375;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

/// One of the KaTeX font files, named as the font system knows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Face {
    /// The family name in the file, `KaTeX_Main`.
    pub family: &'static str,
    pub bold: bool,
    pub italic: bool,
}

/// Why a formula could not be typeset: the engine's message, as KaTeX
/// phrases it (`Undefined control sequence: \foo`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MathError {
    pub message: String,
}

impl std::fmt::Display for MathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for MathError {}

pub type Result = std::result::Result<Arc<Formula>, MathError>;

/// Typesets `tex` in display style (`$$...$$`) or text style (`$...$`).
/// Cached: the second call with the same arguments is a lookup.
pub fn typeset(tex: &str, display: bool) -> Result {
    let key = (tex.to_string(), display);
    if let Some(hit) = cache().lock().ok().and_then(|c| c.get(&key).cloned()) {
        return hit;
    }
    let result = typeset_uncached(tex, display);
    if let Ok(mut c) = cache().lock() {
        // A long session touches many formulas; none of them is worth more
        // than a millisecond to redo, so the cache is simply emptied when it
        // grows large rather than kept in order.
        if c.len() >= CACHE_LIMIT {
            c.clear();
        }
        c.insert(key, result.clone());
    }
    result
}

const CACHE_LIMIT: usize = 4096;

fn cache() -> &'static Mutex<HashMap<(String, bool), Result>> {
    static CACHE: OnceLock<Mutex<HashMap<(String, bool), Result>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn typeset_uncached(tex: &str, display: bool) -> Result {
    let nodes = ratex_parser::parse(tex).map_err(|e| MathError { message: e.message })?;
    let options = LayoutOptions {
        style: if display {
            MathStyle::Display
        } else {
            MathStyle::Text
        },
        ..LayoutOptions::default()
    };
    let list = to_display_list(&layout(&nodes, &options));
    let items = list
        .items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::GlyphPath {
                x,
                y,
                scale,
                font,
                char_code,
                color,
            } => {
                let id = ratex_font::FontId::parse(font)?;
                // Bold and script letters are laid out by their Unicode
                // mathematical alphanumeric code point, but the KaTeX files
                // keep those glyphs at the ASCII letter they style.
                let ch = ratex_font::katex_ttf_glyph_char(id, *char_code);
                Some(Item::Glyph {
                    x: *x,
                    y: *y,
                    size: *scale,
                    face: face_of(id),
                    ch,
                    color: color_of(color),
                })
            }
            DisplayItem::Line {
                x,
                y,
                width,
                thickness,
                color,
                dashed,
            } => Some(Item::Line {
                x: *x,
                y: *y,
                width: *width,
                thickness: *thickness,
                dashed: *dashed,
                color: color_of(color),
            }),
            DisplayItem::Rect {
                x,
                y,
                width,
                height,
                color,
            } => Some(Item::Rect {
                x: *x,
                y: *y,
                width: *width,
                height: *height,
                color: color_of(color),
            }),
            DisplayItem::Path {
                x,
                y,
                commands,
                fill,
                color,
            } => Some(Item::Path {
                x: *x,
                y: *y,
                commands: commands.clone(),
                fill: *fill,
                color: color_of(color),
            }),
        })
        .collect();
    Ok(Arc::new(Formula {
        width: list.width,
        height: list.height,
        depth: list.depth,
        items,
    }))
}

fn color_of(c: &ratex_types::Color) -> Option<Rgba> {
    if *c == ratex_types::Color::BLACK {
        None
    } else {
        Some(Rgba {
            r: c.r,
            g: c.g,
            b: c.b,
            a: c.a,
        })
    }
}

/// The face a RaTeX font id is drawn with. The CJK and emoji fallbacks have
/// no file of their own: they are drawn from the main face, which is what
/// a missing glyph in any face falls through to anyway.
fn face_of(id: ratex_font::FontId) -> Face {
    use ratex_font::FontId::*;
    let (family, bold, italic) = match id {
        AmsRegular => ("KaTeX_AMS", false, false),
        CaligraphicRegular => ("KaTeX_Caligraphic", false, false),
        FrakturRegular => ("KaTeX_Fraktur", false, false),
        FrakturBold => ("KaTeX_Fraktur", true, false),
        MainRegular => ("KaTeX_Main", false, false),
        MainBold => ("KaTeX_Main", true, false),
        MainItalic => ("KaTeX_Main", false, true),
        MainBoldItalic => ("KaTeX_Main", true, true),
        MathItalic => ("KaTeX_Math", false, true),
        MathBoldItalic => ("KaTeX_Math", true, true),
        SansSerifRegular => ("KaTeX_SansSerif", false, false),
        SansSerifBold => ("KaTeX_SansSerif", true, false),
        SansSerifItalic => ("KaTeX_SansSerif", false, true),
        ScriptRegular => ("KaTeX_Script", false, false),
        Size1Regular => ("KaTeX_Size1", false, false),
        Size2Regular => ("KaTeX_Size2", false, false),
        Size3Regular => ("KaTeX_Size3", false, false),
        Size4Regular => ("KaTeX_Size4", false, false),
        TypewriterRegular => ("KaTeX_Typewriter", false, false),
        CjkRegular | CjkFallback | EmojiFallback => ("KaTeX_Main", false, false),
    };
    Face {
        family,
        bold,
        italic,
    }
}

/// The KaTeX font files, SIL Open Font License, embedded at build time;
/// `rasterize` draws from them. They are not handed to the app's font
/// system: gpui's font database drops a face that has no `m` glyph, which
/// is every KaTeX size face, and the italic files carry no italic flag, so
/// a family lookup cannot tell them from the uprights.
pub fn fonts() -> Vec<std::borrow::Cow<'static, [u8]>> {
    FONT_FILES
        .iter()
        .filter_map(|name| ratex_katex_fonts::ttf_bytes(name))
        .collect()
}

/// The file names behind [`fonts`].
pub const FONT_FILES: [&str; 20] = [
    "KaTeX_AMS-Regular.ttf",
    "KaTeX_Caligraphic-Bold.ttf",
    "KaTeX_Caligraphic-Regular.ttf",
    "KaTeX_Fraktur-Bold.ttf",
    "KaTeX_Fraktur-Regular.ttf",
    "KaTeX_Main-Bold.ttf",
    "KaTeX_Main-BoldItalic.ttf",
    "KaTeX_Main-Italic.ttf",
    "KaTeX_Main-Regular.ttf",
    "KaTeX_Math-BoldItalic.ttf",
    "KaTeX_Math-Italic.ttf",
    "KaTeX_SansSerif-Bold.ttf",
    "KaTeX_SansSerif-Italic.ttf",
    "KaTeX_SansSerif-Regular.ttf",
    "KaTeX_Script-Regular.ttf",
    "KaTeX_Size1-Regular.ttf",
    "KaTeX_Size2-Regular.ttf",
    "KaTeX_Size3-Regular.ttf",
    "KaTeX_Size4-Regular.ttf",
    "KaTeX_Typewriter-Regular.ttf",
];

/// The fonts' licence notice, for an About box or a bundle.
pub fn font_notice() -> Option<std::borrow::Cow<'static, [u8]>> {
    ratex_katex_fonts::ttf_bytes("OFL.txt")
}

impl Face {
    /// The file this face is drawn from.
    pub fn file(&self) -> &'static str {
        let style = match (self.bold, self.italic) {
            (false, false) => "Regular",
            (true, false) => "Bold",
            (false, true) => "Italic",
            (true, true) => "BoldItalic",
        };
        let base = &self.family["KaTeX_".len()..];
        FONT_FILES
            .iter()
            .copied()
            .find(|f| *f == format!("KaTeX_{base}-{style}.ttf"))
            .unwrap_or("KaTeX_Main-Regular.ttf")
    }
}

/// A glyph's coverage, rasterized for one size and subpixel offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    /// Offset of the bitmap's top left corner from the glyph's baseline
    /// origin, in device pixels; `top` is negative for anything above the
    /// baseline.
    pub left: i32,
    pub top: i32,
    /// `width * height` coverage values, rows top to bottom.
    pub coverage: Vec<u8>,
}

/// Rasterizes `ch` of `face` at `em_px` device pixels per em, with its
/// origin `subpixel_x` (0 to 1) right of a pixel edge. `None` when the face
/// has no such glyph or it has no ink (a space).
pub fn rasterize(face: Face, ch: char, em_px: f32, subpixel_x: f32) -> Option<Bitmap> {
    use ab_glyph::{Font, PxScale};
    let font = parsed_font(face)?;
    let id = font.glyph_id(ch);
    if id.0 == 0 {
        return None;
    }
    // ab_glyph scales by the font's height (ascent minus descent), not its
    // em; convert so that one em is `em_px`.
    let per_em = font.units_per_em()?;
    let scale = PxScale::from(em_px * font.height_unscaled() / per_em);
    let glyph = id.with_scale_and_position(scale, ab_glyph::point(subpixel_x, 0.0));
    let outlined = font.outline_glyph(glyph)?;
    let bounds = outlined.px_bounds();
    let width = (bounds.max.x - bounds.min.x).max(0.0) as u32;
    let height = (bounds.max.y - bounds.min.y).max(0.0) as u32;
    if width == 0 || height == 0 {
        return None;
    }
    let mut coverage = vec![0u8; (width * height) as usize];
    outlined.draw(|x, y, c| {
        if x < width && y < height {
            coverage[(y * width + x) as usize] = (c.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    });
    Some(Bitmap {
        width,
        height,
        left: bounds.min.x as i32,
        top: bounds.min.y as i32,
        coverage,
    })
}

/// The horizontal advance of `ch` in `face`, in em; `None` when the face
/// lacks the glyph.
pub fn advance(face: Face, ch: char) -> Option<f64> {
    use ab_glyph::Font;
    let font = parsed_font(face)?;
    let id = font.glyph_id(ch);
    if id.0 == 0 {
        return None;
    }
    Some(font.h_advance_unscaled(id) as f64 / font.units_per_em()? as f64)
}

fn parsed_font(face: Face) -> Option<&'static ab_glyph::FontRef<'static>> {
    static FONTS: OnceLock<
        Mutex<HashMap<&'static str, Option<&'static ab_glyph::FontRef<'static>>>>,
    > = OnceLock::new();
    let fonts = FONTS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut fonts = fonts.lock().ok()?;
    let file = face.file();
    *fonts.entry(file).or_insert_with(|| {
        // The bytes are embedded in release builds and read from disk in
        // debug ones; either way they live for the rest of the process.
        let bytes: &'static [u8] = match ratex_katex_fonts::ttf_bytes(file)? {
            std::borrow::Cow::Borrowed(b) => b,
            std::borrow::Cow::Owned(v) => Box::leak(v.into_boxed_slice()),
        };
        let font = ab_glyph::FontRef::try_from_slice(bytes).ok()?;
        Some(Box::leak(Box::new(font)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fraction_has_two_glyphs_and_a_bar() {
        let f = typeset(r"\frac{a}{b}", true).unwrap();
        let glyphs = f
            .items
            .iter()
            .filter(|i| matches!(i, Item::Glyph { .. }))
            .count();
        let bars = f
            .items
            .iter()
            .filter(|i| matches!(i, Item::Line { .. }))
            .count();
        assert_eq!((glyphs, bars), (2, 1));
        assert!(f.width > 0.4 && f.width < 1.0, "width {}", f.width);
        // The numerator sits above the baseline, the denominator below it.
        assert!(
            f.height > 0.9 && f.depth > 0.5,
            "{} / {}",
            f.height,
            f.depth
        );
    }

    #[test]
    fn text_style_keeps_a_sum_small() {
        let display = typeset(r"\sum_{i=0}^n i", true).unwrap();
        let inline = typeset(r"\sum_{i=0}^n i", false).unwrap();
        assert!(inline.total_height() < display.total_height());
    }

    #[test]
    fn letters_are_italic_and_digits_upright() {
        let f = typeset("x2", false).unwrap();
        let faces: Vec<_> = f
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Glyph { face, ch, .. } => Some((*ch, *face)),
                _ => None,
            })
            .collect();
        assert_eq!(
            faces,
            vec![
                (
                    'x',
                    Face {
                        family: "KaTeX_Math",
                        bold: false,
                        italic: true
                    }
                ),
                (
                    '2',
                    Face {
                        family: "KaTeX_Main",
                        bold: false,
                        italic: false
                    }
                ),
            ]
        );
    }

    #[test]
    fn a_bold_letter_is_drawn_from_the_ascii_slot() {
        let f = typeset(r"\mathbf{A}", false).unwrap();
        let Some(Item::Glyph { face, ch, .. }) = f.items.first() else {
            panic!("no glyph");
        };
        assert_eq!(*ch, 'A');
        assert!(face.bold);
    }

    #[test]
    fn a_radical_is_a_path_or_a_size_glyph() {
        let f = typeset(r"\sqrt{\frac{a}{b}}", true).unwrap();
        assert!(f
            .items
            .iter()
            .any(|i| matches!(i, Item::Path { .. } | Item::Glyph { ch: '\u{221a}', .. })));
    }

    #[test]
    fn colour_is_none_for_black_and_set_otherwise() {
        let f = typeset(r"\textcolor{red}{x} y", false).unwrap();
        let colours: Vec<_> = f
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Glyph { ch, color, .. } => Some((*ch, color.is_some())),
                _ => None,
            })
            .collect();
        assert_eq!(colours, vec![('x', true), ('y', false)]);
    }

    #[test]
    fn an_unclosed_brace_is_an_error_with_a_message() {
        let e = typeset(r"\frac{a}{", true).unwrap_err();
        assert!(!e.message.is_empty());
        assert_eq!(typeset(r"\frac{a}{", true).unwrap_err(), e);
    }

    #[test]
    fn an_undefined_command_is_an_error() {
        let e = typeset(r"\nosuchcommand", false).unwrap_err();
        assert!(e.message.contains("nosuchcommand"), "{}", e.message);
    }

    #[test]
    fn the_cache_hands_back_the_same_formula() {
        let a = typeset("a+b", false).unwrap();
        let b = typeset("a+b", false).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        assert!(!Arc::ptr_eq(&a, &typeset("a+b", true).unwrap()));
    }

    #[test]
    fn every_font_file_is_embedded_truetype() {
        let fonts = fonts();
        assert_eq!(fonts.len(), FONT_FILES.len());
        for f in fonts {
            assert_eq!(&f[..4], &[0, 1, 0, 0]);
        }
        assert!(font_notice().is_some());
    }

    #[test]
    fn every_face_rasterizes_and_the_size_faces_are_tall() {
        let main = Face {
            family: "KaTeX_Main",
            bold: false,
            italic: false,
        };
        let x = rasterize(main, 'x', 16.0, 0.0).unwrap();
        assert!(x.width >= 6 && x.width <= 10, "{}", x.width);
        assert!(
            x.top < 0 && x.top + x.height as i32 <= 1,
            "{} {}",
            x.top,
            x.height
        );
        assert!(x.coverage.iter().any(|&c| c > 128));
        // The italic file is a different drawing from the upright one.
        let italic = rasterize(
            Face {
                italic: true,
                ..main
            },
            'x',
            16.0,
            0.0,
        )
        .unwrap();
        assert_ne!(italic.coverage, x.coverage);
        // A size face holds the big parenthesis the matrix needs.
        let paren = rasterize(
            Face {
                family: "KaTeX_Size3",
                bold: false,
                italic: false,
            },
            '(',
            16.0,
            0.0,
        )
        .unwrap();
        assert!(paren.height > 30, "{}", paren.height);
        // And a space has no ink.
        assert!(rasterize(main, ' ', 16.0, 0.0).is_none());
        assert!(advance(main, 'x').is_some_and(|a| a > 0.4 && a < 0.6));
    }

    #[test]
    fn a_face_names_its_file() {
        let f = |family, bold, italic| {
            Face {
                family,
                bold,
                italic,
            }
            .file()
        };
        assert_eq!(f("KaTeX_Main", true, true), "KaTeX_Main-BoldItalic.ttf");
        assert_eq!(f("KaTeX_Math", false, true), "KaTeX_Math-Italic.ttf");
        assert_eq!(f("KaTeX_Size2", false, false), "KaTeX_Size2-Regular.ttf");
        assert_eq!(
            f("KaTeX_Caligraphic", true, false),
            "KaTeX_Caligraphic-Bold.ttf"
        );
    }

    #[test]
    fn an_empty_formula_typesets_to_nothing() {
        let f = typeset("", false).unwrap();
        assert!(f.items.is_empty());
    }

    #[test]
    fn environments_and_mhchem_parse() {
        for tex in [
            r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}",
            r"\begin{aligned} a &= b \\ c &= d \end{aligned}",
            r"\ce{H2O}",
            r"\left( \sum_{i=0}^{n} i \right)",
            r"\hat{x} \vec{v} \overline{z} \underbrace{abc}_{3}",
            r"\text{if } x > 0",
        ] {
            typeset(tex, true).unwrap_or_else(|e| panic!("{tex}: {e}"));
        }
    }
}
