//! Formulas on screen: paints what `silica-math` typesets.
//!
//! A formula arrives as glyphs, rules and paths in em units. This module
//! scales them to the font size of the text around them and paints them in
//! that text's colour (a `\textcolor` keeps its own): the glyphs as bitmaps
//! `silica-math` rasterizes from the KaTeX faces, placed on the device
//! pixel grid, and radicals and stretched delimiters through gpui's path
//! tessellator. The faces never reach gpui's font system, whose database
//! drops a face with no `m` glyph (every KaTeX size face) and cannot tell
//! the KaTeX italics from the uprights. Three callers share it: the reading view's
//! block and inline math, Live Preview's inline math painted over its
//! placeholder, and the math block widget, which is the reading view's.
//!
//! Inline math sits inside shaped text, and gpui shapes text only: there
//! is no room in a line for an element. So an inline formula is laid out
//! as a run of no-break spaces exactly as wide as it needs (`placeholder`),
//! which wraps and selects like text, and the formula is painted over the
//! run afterwards. Figure space, no-break space and narrow no-break space
//! are the three widths; the run overshoots the formula by less than the
//! narrowest of them and the formula is centred in the difference.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use std::ops::Range;

use gpui::{
    fill, point, px, rgb, size, AnyElement, App, Bounds, Corners, Element, ElementId, Font,
    GlobalElementId, Hsla, InspectorElementId, IntoElement, LayoutId, PathBuilder, Pixels, Point,
    Style, TextLayout, TextSystem, Window,
};
use silica_math::{Face, Formula, Item, PathCommand, Rgba, STROKE_WIDTH};

pub use silica_math::typeset;

/// What a formula that does not parse is shown in: its source, in the red
/// MathJax gives its error output (Obsidian's `--text-error`).
pub fn error_color() -> Hsla {
    rgb(0xe93147).into()
}

/// The pixels of a formula set at `em` pixels per em.
pub fn width_px(formula: &Formula, em: Pixels) -> Pixels {
    em * formula.width as f32
}

pub fn height_px(formula: &Formula, em: Pixels) -> Pixels {
    em * formula.height as f32
}

pub fn depth_px(formula: &Formula, em: Pixels) -> Pixels {
    em * formula.depth as f32
}

/// Paints `formula` with its baseline's left end at `baseline`, `em`
/// pixels per em, in `color` where the formula names none.
pub fn paint(
    formula: &Formula,
    baseline: Point<Pixels>,
    em: Pixels,
    color: Hsla,
    window: &mut Window,
) {
    let origin = point(baseline.x, baseline.y - height_px(formula, em));
    let scale = |v: f64| em * v as f32;
    let at = |x: f64, y: f64| point(origin.x + scale(x), origin.y + scale(y));
    let colour = |c: &Option<Rgba>| match c {
        Some(c) => gpui::Rgba {
            r: c.r,
            g: c.g,
            b: c.b,
            a: c.a,
        }
        .into(),
        None => color,
    };
    for item in &formula.items {
        match item {
            Item::Glyph {
                x,
                y,
                size,
                face,
                ch,
                color,
            } => {
                paint_glyph(window, *face, *ch, at(*x, *y), scale(*size), colour(color));
            }
            Item::Line {
                x,
                y,
                width,
                thickness,
                dashed,
                color,
            } => {
                // The rule is drawn no thinner than a device pixel, which
                // is what a browser does with a fraction bar too.
                let thickness = scale(*thickness).max(px(1.) / window.scale_factor());
                let colour = colour(color);
                if *dashed {
                    let dash = scale(0.12);
                    let gap = scale(0.08);
                    let mut from = px(0.);
                    let total = scale(*width);
                    while from < total {
                        let to = (from + dash).min(total);
                        let mut p = at(*x, *y);
                        p.x += from;
                        window.paint_quad(fill(Bounds::new(p, size(to - from, thickness)), colour));
                        from = to + gap;
                    }
                } else {
                    window.paint_quad(fill(
                        Bounds::new(at(*x, *y), size(scale(*width), thickness)),
                        colour,
                    ));
                }
            }
            Item::Rect {
                x,
                y,
                width,
                height,
                color,
            } => {
                window.paint_quad(fill(
                    Bounds::new(at(*x, *y), size(scale(*width), scale(*height))),
                    colour(color),
                ));
            }
            Item::Path {
                x,
                y,
                commands,
                fill: filled,
                color,
            } => {
                let mut builder = if *filled {
                    PathBuilder::fill()
                } else {
                    PathBuilder::stroke(scale(STROKE_WIDTH).max(px(1.)))
                };
                let base = at(*x, *y);
                let rel = |cx: f64, cy: f64| point(base.x + scale(cx), base.y + scale(cy));
                let mut open = false;
                for command in commands {
                    match *command {
                        PathCommand::MoveTo { x, y } => {
                            builder.move_to(rel(x, y));
                            open = true;
                        }
                        PathCommand::LineTo { x, y } => builder.line_to(rel(x, y)),
                        PathCommand::QuadTo { x1, y1, x, y } => {
                            builder.curve_to(rel(x, y), rel(x1, y1))
                        }
                        PathCommand::CubicTo {
                            x1,
                            y1,
                            x2,
                            y2,
                            x,
                            y,
                        } => builder.cubic_bezier_to(rel(x, y), rel(x1, y1), rel(x2, y2)),
                        PathCommand::Close => {
                            if open {
                                builder.close();
                            }
                            open = false;
                        }
                    }
                }
                if let Ok(path) = builder.build() {
                    window.paint_path(path, colour(color));
                }
            }
        }
    }
}

/// The bitmap of one glyph at one device size, subpixel offset and colour:
/// the text colour with the glyph's coverage as alpha, straight (not
/// premultiplied) BGRA as gpui takes its images. `None` is cached too, for
/// a glyph with no ink.
#[derive(Clone, PartialEq, Eq, Hash)]
struct GlyphKey {
    face: Face,
    ch: char,
    /// Device pixels per em, in sixteenths.
    em_16: u32,
    /// Which quarter of a device pixel the origin sits in.
    subpixel: u8,
    color: u32,
}

const SUBPIXEL_VARIANTS: f32 = 4.0;

struct GlyphImage {
    image: Arc<gpui::RenderImage>,
    left: i32,
    top: i32,
    width: u32,
    height: u32,
}

fn glyph_image(key: &GlyphKey) -> Option<Arc<GlyphImage>> {
    type Cache = HashMap<GlyphKey, Option<Arc<GlyphImage>>>;
    thread_local! {
        static GLYPHS: RefCell<Cache> = RefCell::new(HashMap::new());
    }
    if let Some(hit) = GLYPHS.with(|g| g.borrow().get(key).cloned()) {
        return hit;
    }
    let bitmap = silica_math::rasterize(
        key.face,
        key.ch,
        key.em_16 as f32 / 16.0,
        key.subpixel as f32 / SUBPIXEL_VARIANTS,
    );
    let built = bitmap.map(|b| {
        let [r, g, bl, _] = key.color.to_be_bytes();
        let mut bytes = Vec::with_capacity(b.coverage.len() * 4);
        for &c in &b.coverage {
            bytes.extend_from_slice(&[bl, g, r, c]);
        }
        let buffer = image::RgbaImage::from_raw(b.width, b.height, bytes)
            .expect("the buffer is width * height * 4 bytes");
        Arc::new(GlyphImage {
            image: Arc::new(gpui::RenderImage::new(vec![image::Frame::new(buffer)])),
            left: b.left,
            top: b.top,
            width: b.width,
            height: b.height,
        })
    });
    GLYPHS.with(|g| {
        let mut g = g.borrow_mut();
        // A session that changes sizes and themes grows this without bound
        // otherwise; nothing in it costs more than a millisecond to redo.
        if g.len() > 4096 {
            g.clear();
        }
        g.insert(key.clone(), built.clone());
    });
    built
}

/// Paints one glyph with its baseline origin at `origin`, `em` logical
/// pixels per em. The bitmap is made for the device pixel grid: the origin
/// is snapped to a quarter pixel horizontally and a whole pixel vertically,
/// and the image is placed so its pixels land on the grid one to one.
fn paint_glyph(
    window: &mut Window,
    face: Face,
    ch: char,
    origin: Point<Pixels>,
    em: Pixels,
    color: Hsla,
) {
    let scale = window.scale_factor();
    let device_x = f32::from(origin.x) * scale;
    let device_y = f32::from(origin.y) * scale;
    let mut pixel_x = device_x.floor();
    let mut subpixel = ((device_x - pixel_x) * SUBPIXEL_VARIANTS).round() as u8;
    if subpixel as f32 >= SUBPIXEL_VARIANTS {
        subpixel = 0;
        pixel_x += 1.0;
    }
    let pixel_y = device_y.round();
    let rgba: gpui::Rgba = color.into();
    let color = u32::from_be_bytes([
        (rgba.r * 255.0).round() as u8,
        (rgba.g * 255.0).round() as u8,
        (rgba.b * 255.0).round() as u8,
        (rgba.a * 255.0).round() as u8,
    ]);
    let key = GlyphKey {
        face,
        ch,
        em_16: (f32::from(em) * scale * 16.0).round() as u32,
        subpixel,
        color,
    };
    let Some(glyph) = glyph_image(&key) else {
        return;
    };
    let bounds = Bounds::new(
        point(
            px((pixel_x + glyph.left as f32) / scale),
            px((pixel_y + glyph.top as f32) / scale),
        ),
        size(
            px(glyph.width as f32 / scale),
            px(glyph.height as f32 / scale),
        ),
    );
    let _ = window.paint_image(bounds, Corners::default(), glyph.image.clone(), 0, false);
}

// ---------------------------------------------------------------------------
// Inline placeholders

/// The advance of each no-break space the text font has, at one size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpaceWidths {
    /// U+2007 FIGURE SPACE, a digit wide.
    figure: Pixels,
    /// U+00A0 NO-BREAK SPACE, a word space wide.
    nbsp: Pixels,
    /// U+202F NARROW NO-BREAK SPACE.
    narrow: Pixels,
}

impl SpaceWidths {
    /// Measures the three spaces in `font` at `size`. A space the font
    /// lacks measures zero and is never used, because the glyph shaping
    /// would borrow for it has a width this cannot know.
    pub fn measure(text_system: &TextSystem, font: &Font, size: Pixels) -> SpaceWidths {
        let font_id = text_system.resolve_font(font);
        let advance = |ch: char| {
            text_system
                .advance(font_id, size, ch)
                .map(|s| s.width)
                .unwrap_or(px(0.))
        };
        SpaceWidths {
            figure: advance('\u{2007}'),
            nbsp: advance('\u{a0}'),
            narrow: advance('\u{202f}'),
        }
    }

    /// A run of no-break spaces at least `width` wide, and its width. The
    /// run is never empty, so a formula with no ink still has somewhere to
    /// be clicked.
    pub fn placeholder(&self, width: Pixels) -> (String, Pixels) {
        let mut out = String::new();
        let mut total = px(0.);
        let mut remaining = width;
        let mut units: Vec<(char, Pixels)> = vec![
            ('\u{2007}', self.figure),
            ('\u{a0}', self.nbsp),
            ('\u{202f}', self.narrow),
        ];
        units.retain(|(_, w)| *w > px(0.));
        units.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        if units.is_empty() {
            return ("\u{a0}".to_string(), width);
        }
        for (ch, w) in &units {
            while remaining >= *w {
                out.push(*ch);
                total += *w;
                remaining -= *w;
            }
        }
        if remaining > px(0.) || out.is_empty() {
            let (ch, w) = units[units.len() - 1];
            out.push(ch);
            total += w;
        }
        (out, total)
    }
}

/// How much taller than its text a line with this inline formula must be:
/// zero when the formula fits inside the font's ascent and descent.
pub fn inline_overflow(formula: &Formula, em: Pixels, ascent: Pixels, descent: Pixels) -> Pixels {
    let above = (height_px(formula, em) - ascent).max(px(0.));
    let below = (depth_px(formula, em) - descent).max(px(0.));
    above.max(below) * 2.
}

// ---------------------------------------------------------------------------
// Element

/// A formula as a fixed-size element: the reading view's `$$` block and
/// the math block widget, laid out by the box it stands in.
pub struct FormulaElement {
    formula: Arc<Formula>,
    em: Pixels,
    color: Hsla,
}

impl FormulaElement {
    pub fn new(formula: Arc<Formula>, em: Pixels, color: Hsla) -> Self {
        FormulaElement { formula, em, color }
    }
}

impl IntoElement for FormulaElement {
    type Element = FormulaElement;

    fn into_element(self) -> FormulaElement {
        self
    }
}

impl Element for FormulaElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = width_px(&self.formula, self.em).into();
        style.size.height =
            (height_px(&self.formula, self.em) + depth_px(&self.formula, self.em)).into();
        style.flex_shrink = 0.;
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        _prepaint: &mut (),
        window: &mut Window,
        _cx: &mut App,
    ) {
        let baseline = point(
            bounds.origin.x,
            bounds.origin.y + height_px(&self.formula, self.em),
        );
        paint(&self.formula, baseline, self.em, self.color, window);
    }
}

/// An inline formula of a shaped text: where its placeholder is in the
/// string, what to paint there, and how wide the placeholder came out.
pub struct Inline {
    pub range: Range<usize>,
    pub formula: Arc<Formula>,
    pub slot: Pixels,
}

/// Shaped text with formulas painted over its placeholders: wraps the
/// text element and, once that has laid out, reads where each placeholder
/// landed and paints the formula on the row's baseline, centred in the
/// placeholder's slack.
pub struct Overlay {
    child: AnyElement,
    layout: TextLayout,
    inlines: Vec<Inline>,
    em: Pixels,
    color: Hsla,
}

impl Overlay {
    pub fn new(
        child: AnyElement,
        layout: TextLayout,
        inlines: Vec<Inline>,
        em: Pixels,
        color: Hsla,
    ) -> Overlay {
        Overlay {
            child,
            layout,
            inlines,
            em,
            color,
        }
    }
}

impl IntoElement for Overlay {
    type Element = Overlay;

    fn into_element(self) -> Overlay {
        self
    }
}

impl Element for Overlay {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        _prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
        let line_height = self.layout.line_height();
        for inline in &self.inlines {
            let Some(start) = self.layout.position_for_index(inline.range.start) else {
                continue;
            };
            let Some(line) = self.layout.line_layout_for_index(inline.range.start) else {
                continue;
            };
            let (ascent, descent) = (line.unwrapped_layout.ascent, line.unwrapped_layout.descent);
            let width = width_px(&inline.formula, self.em);
            // The placeholder never breaks, so its end is on the same row
            // unless the text system disagreed about the width; then the
            // formula simply starts where the placeholder does.
            let end_x = self
                .layout
                .position_for_index(inline.range.end)
                .filter(|end| end.y == start.y)
                .map(|end| end.x)
                .unwrap_or(start.x + inline.slot);
            let slack = (end_x - start.x - width).max(px(0.));
            let baseline = point(
                start.x + slack / 2.,
                start.y + (line_height - ascent - descent) / 2. + ascent,
            );
            paint(&inline.formula, baseline, self.em, self.color, window);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_placeholder_is_at_least_as_wide_as_asked_and_never_empty() {
        let w = SpaceWidths {
            figure: px(10.),
            nbsp: px(4.),
            narrow: px(2.),
        };
        let (s, total) = w.placeholder(px(27.));
        // 2 figure (20) + 1 nbsp (24) + 1 narrow (26) + 1 narrow to cover.
        assert_eq!(s, "\u{2007}\u{2007}\u{a0}\u{202f}\u{202f}");
        assert_eq!(total, px(28.));
        let (s, total) = w.placeholder(px(0.));
        assert_eq!((s.as_str(), total), ("\u{202f}", px(2.)));
        assert!(s.chars().all(|c| c != ' '));
    }

    #[test]
    fn a_font_without_the_narrow_spaces_uses_the_ones_it_has() {
        let w = SpaceWidths {
            figure: px(0.),
            nbsp: px(4.),
            narrow: px(0.),
        };
        let (s, total) = w.placeholder(px(9.));
        assert_eq!(s, "\u{a0}\u{a0}\u{a0}");
        assert_eq!(total, px(12.));
        let none = SpaceWidths {
            figure: px(0.),
            nbsp: px(0.),
            narrow: px(0.),
        };
        assert_eq!(none.placeholder(px(9.)).0, "\u{a0}");
    }

    #[test]
    fn overflow_is_zero_inside_the_text_box() {
        let f = typeset("x", false).unwrap();
        assert_eq!(inline_overflow(&f, px(16.), px(15.), px(4.)), px(0.));
        let tall = typeset(r"\frac{a}{b}", false).unwrap();
        assert!(inline_overflow(&tall, px(16.), px(15.), px(4.)) > px(0.));
    }
}
