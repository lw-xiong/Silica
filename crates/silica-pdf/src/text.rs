//! Page text for find: a `Device` that keeps the unicode of every glyph run and
//! assembles it in content order, with a space on a horizontal gap and a
//! newline on a vertical move.

use hayro::hayro_interpret::font::{Glyph, GlyphRun};
use hayro::hayro_interpret::hayro_cmap::BfString;
use hayro::hayro_interpret::hayro_syntax::page::Page;
use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_interpret::{
    interpret_page, BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image,
    ImageDrawProps, InterpreterCache, InterpreterSettings, SoftMask,
};
use hayro::kurbo::{BezPath, Point, Rect};

/// A glyph's text with where it sits on the displayed page, in points.
struct Placed {
    text: String,
    x: f64,
    /// The glyph's right edge, from its advance; the origin plus half a size
    /// when the font has no advance for it.
    right: f64,
    y: f64,
    /// The font size on the page, a yardstick for gaps.
    size: f64,
}

#[derive(Default)]
struct TextDevice {
    glyphs: Vec<Placed>,
}

impl<'a> Device<'a> for TextDevice {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}

    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, _: &DrawMode) {
        for glyph in run.glyphs() {
            let Some(unicode) = glyph.as_unicode() else {
                continue;
            };
            let text = match unicode {
                BfString::Char(c) => c.to_string(),
                BfString::String(s) => s,
            };
            // The glyph transform maps 1000-unit glyph space to the page, so
            // its scale is the font size over 1000.
            let to_page = props.transform * glyph.transform();
            let origin = to_page * Point::ZERO;
            let size = (to_page.determinant().abs().sqrt() * 1000.0).max(0.1);
            let advance = match &**glyph {
                Glyph::Outline(outline) => outline.advance_width().map(f64::from),
                Glyph::Type3(_) => None,
            };
            let right = match advance {
                Some(advance) => (to_page * Point::new(advance, 0.0)).x,
                None => origin.x + size / 2.0,
            };
            self.glyphs.push(Placed {
                text,
                x: origin.x,
                right,
                y: origin.y,
                size,
            });
        }
    }
}

pub(crate) fn page_text(page: &Page<'_>, settings: &InterpreterSettings) -> String {
    let (width, height) = page.render_dimensions();
    let transform = page.initial_transform(true).to_kurbo();
    let cache = InterpreterCache::new();
    let mut context = Context::new(
        transform,
        Rect::new(0.0, 0.0, width as f64, height as f64),
        &cache,
        page.xref(),
        settings.clone(),
    );
    let mut device = TextDevice::default();
    interpret_page(page, &mut context, &mut device);
    assemble(&device.glyphs)
}

fn assemble(glyphs: &[Placed]) -> String {
    let mut out = String::new();
    let mut previous: Option<&Placed> = None;
    for glyph in glyphs {
        if let Some(last) = previous {
            let size = last.size.max(glyph.size);
            let moved_line = (glyph.y - last.y).abs() > size * 0.5;
            let went_back = glyph.x < last.x - size;
            if moved_line || went_back {
                trim_trailing_space(&mut out);
                out.push('\n');
            } else if glyph.x - last.right > size * 0.15 && !ends_with_space(&out) {
                out.push(' ');
            }
        }
        if glyph.text.chars().all(char::is_whitespace) {
            if !ends_with_space(&out) && !out.is_empty() {
                out.push(' ');
            }
        } else {
            out.push_str(&glyph.text);
        }
        previous = Some(glyph);
    }
    trim_trailing_space(&mut out);
    out
}

fn ends_with_space(s: &str) -> bool {
    s.ends_with([' ', '\n'])
}

fn trim_trailing_space(s: &mut String) {
    while s.ends_with(' ') {
        s.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::{assemble, Placed};

    fn glyph(text: &str, x: f64, y: f64) -> Placed {
        Placed {
            text: text.to_string(),
            x,
            right: x + 5.0,
            y,
            size: 10.0,
        }
    }

    #[test]
    fn spaces_on_gaps_and_newlines_on_moves() {
        let glyphs = vec![
            glyph("H", 0.0, 100.0),
            glyph("i", 5.0, 100.0),
            glyph("t", 14.0, 100.0),
            glyph("o", 19.0, 100.0),
            glyph("y", 0.0, 112.0),
            glyph("o", 5.0, 112.0),
            glyph("u", 10.0, 112.0),
        ];
        assert_eq!(assemble(&glyphs), "Hi to\nyou");
    }

    #[test]
    fn explicit_space_glyphs_collapse() {
        let glyphs = vec![
            glyph("a", 0.0, 0.0),
            glyph(" ", 5.0, 0.0),
            glyph("b", 12.0, 0.0),
            glyph(" ", 17.0, 0.0),
        ];
        assert_eq!(assemble(&glyphs), "a b");
    }

    #[test]
    fn empty_page() {
        assert_eq!(assemble(&[]), "");
    }
}
