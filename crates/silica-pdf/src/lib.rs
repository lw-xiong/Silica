//! PDF documents for Silica's viewer and `![[x.pdf]]` embeds: page sizes, rendered
//! pages, the outline, the title, and best-effort page text for find.
//!
//! Rendering is [`hayro`], a pure Rust rasterizer. A [`Document`] is `Send + Sync`,
//! so the view can render pages on a background thread while the UI thread reads
//! sizes. Every entry point guards against a panic inside the renderer: malformed
//! input gives an `Err` or a `None`, never a crash.

#![forbid(unsafe_code)]

mod outline;
mod subpath;
mod text;
mod textstring;

use std::fmt;
use std::panic::{catch_unwind, AssertUnwindSafe};

use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::{DecryptionError, LoadPdfError, Pdf};
use hayro::kurbo::Affine;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::vello_cpu::{Pixmap, RasterizerSettings, RenderContext, Resources, TargetInit};
use hayro::{render_into, RenderCache, RenderSettings};

pub use subpath::{parse_subpath, Subpath};

/// The longest side a rendered page may have, in pixels. A larger request is
/// scaled down to fit; the bitmap then reports the size it was actually drawn at.
pub const MAX_BITMAP_SIDE: u32 = 8192;

/// A parsed PDF file. Owns its bytes.
pub struct Document {
    pdf: Pdf,
    settings: InterpreterSettings,
}

/// A page's size in points (1/72 in), as displayed: after the page's `/Rotate`
/// and its crop box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSize {
    pub width: f32,
    pub height: f32,
}

/// A rendered page: straight (non-premultiplied) RGBA8, row-major, composited
/// over opaque white, so every alpha is 255.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// One entry of the document outline (bookmarks). `page` is zero-based and
/// `None` when the entry's destination could not be resolved; `depth` is 0 at
/// the top level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineItem {
    pub title: String,
    pub page: Option<usize>,
    pub depth: usize,
}

/// Why a file would not open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The bytes are not a PDF hayro can read.
    Parse(String),
    /// The file needs a password.
    Encrypted,
    /// The file has no pages.
    Empty,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Parse(why) => write!(f, "cannot parse the PDF: {why}"),
            Error::Encrypted => f.write_str("the PDF needs a password"),
            Error::Empty => f.write_str("the PDF has no pages"),
        }
    }
}

impl std::error::Error for Error {}

impl Document {
    /// Parse a PDF from its bytes.
    pub fn open(bytes: Vec<u8>) -> Result<Document, Error> {
        let parsed = catch_unwind(AssertUnwindSafe(|| Pdf::new(bytes)))
            .map_err(|_| Error::Parse("the parser panicked".to_string()))?;
        let pdf = match parsed {
            Ok(pdf) => pdf,
            Err(LoadPdfError::Decryption(DecryptionError::PasswordProtected)) => {
                return Err(Error::Encrypted)
            }
            Err(LoadPdfError::Decryption(why)) => {
                return Err(Error::Parse(format!("unsupported encryption: {why:?}")))
            }
            Err(LoadPdfError::Invalid) => {
                return Err(Error::Parse(
                    "not a PDF, or too damaged to read".to_string(),
                ))
            }
        };
        if pdf.pages().is_empty() {
            return Err(Error::Empty);
        }
        Ok(Document {
            pdf,
            settings: InterpreterSettings::default(),
        })
    }

    /// The number of pages.
    pub fn page_count(&self) -> usize {
        self.pdf.pages().len()
    }

    /// The displayed size of a page, or `None` past the last page.
    pub fn page_size(&self, index: usize) -> Option<PageSize> {
        let page = self.pdf.pages().get(index)?;
        let (width, height) = page.render_dimensions();
        Some(PageSize { width, height })
    }

    /// Every page's displayed size, in order.
    pub fn sizes(&self) -> Vec<PageSize> {
        self.pdf
            .pages()
            .iter()
            .map(|page| {
                let (width, height) = page.render_dimensions();
                PageSize { width, height }
            })
            .collect()
    }

    /// Render a page. `scale` 1.0 is one pixel per point; the bitmap is
    /// `ceil(size * scale)` on each side, scaled down if a side would pass
    /// [`MAX_BITMAP_SIDE`]. `None` past the last page, for a scale that is not
    /// positive and finite, or when the renderer gives up on the page.
    pub fn render(&self, index: usize, scale: f32) -> Option<Bitmap> {
        let page = self.pdf.pages().get(index)?;
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let (page_width, page_height) = page.render_dimensions();
        let scale = fit_scale(page_width, page_height, scale);
        let width = ((page_width * scale).ceil() as u32).clamp(1, MAX_BITMAP_SIDE);
        let height = ((page_height * scale).ceil() as u32).clamp(1, MAX_BITMAP_SIDE);

        let rendered = catch_unwind(AssertUnwindSafe(|| {
            // The cache holds `Rc`s, so it lives for this call only.
            let cache = RenderCache::new();
            let mut ctx = RenderContext::new(width as u16, height as u16);
            let transform = Affine::scale(scale as f64) * page.initial_transform(true).to_kurbo();
            render_into(
                page,
                &cache,
                &self.settings,
                &RenderSettings::default(),
                &mut ctx,
                transform,
            );
            ctx.flush();
            let mut pixmap = Pixmap::new(width as u16, height as u16);
            ctx.render_with(
                &mut pixmap,
                &mut Resources::default(),
                RasterizerSettings {
                    target_init: TargetInit::Clear(WHITE),
                    ..Default::default()
                },
            );
            pixmap
        }))
        .ok()?;

        Some(straight_rgba(rendered, width, height))
    }

    /// The document outline (bookmarks), flattened in document order; empty
    /// when the file has none.
    pub fn outline(&self) -> Vec<OutlineItem> {
        catch_unwind(AssertUnwindSafe(|| outline::outline(&self.pdf))).unwrap_or_default()
    }

    /// The Info dictionary's `/Title`, decoded from PDFDocEncoding or UTF-16
    /// with a byte order mark; `None` when absent or blank.
    pub fn title(&self) -> Option<String> {
        let bytes = self.pdf.metadata().title.as_deref()?;
        let title = textstring::decode(bytes);
        let title = title.trim();
        (!title.is_empty()).then(|| title.to_string())
    }

    /// The page's text in reading order, best effort, for find. `Some("")`
    /// when the page draws no text; `None` past the last page or when the
    /// interpreter gives up on the page.
    pub fn page_text(&self, index: usize) -> Option<String> {
        let page = self.pdf.pages().get(index)?;
        catch_unwind(AssertUnwindSafe(|| text::page_text(page, &self.settings))).ok()
    }
}

/// The scale that keeps both sides at or under [`MAX_BITMAP_SIDE`].
fn fit_scale(width: f32, height: f32, scale: f32) -> f32 {
    let max = MAX_BITMAP_SIDE as f32;
    let longest = width.max(height).max(1.0);
    if longest * scale > max {
        max / longest
    } else {
        scale
    }
}

/// vello's pixmap is premultiplied RGBA8; over an opaque background the two
/// agree, but unpremultiply anyway and pin alpha so a blend mode cannot leak
/// transparency into the view.
fn straight_rgba(pixmap: Pixmap, width: u32, height: u32) -> Bitmap {
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for pixel in pixmap.data() {
        let alpha = pixel.a;
        let straight =
            |channel: u8| -> u8 {
                match alpha {
                    255 => channel,
                    0 => 255,
                    _ => ((u32::from(channel) * 255 + u32::from(alpha) / 2) / u32::from(alpha))
                        .min(255) as u8,
                }
            };
        rgba.extend_from_slice(&[straight(pixel.r), straight(pixel.g), straight(pixel.b), 255]);
    }
    debug_assert_eq!(rgba.len(), (width * height * 4) as usize);
    Bitmap {
        width,
        height,
        rgba,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_scale_caps_the_longest_side() {
        assert_eq!(fit_scale(100.0, 50.0, 2.0), 2.0);
        let scale = fit_scale(1000.0, 500.0, 10.0);
        assert!((1000.0 * scale - MAX_BITMAP_SIDE as f32).abs() < 0.01);
        let scale = fit_scale(500.0, 1000.0, 10.0);
        assert!((1000.0 * scale - MAX_BITMAP_SIDE as f32).abs() < 0.01);
    }

    #[test]
    fn errors_display() {
        assert_eq!(Error::Encrypted.to_string(), "the PDF needs a password");
        assert_eq!(Error::Empty.to_string(), "the PDF has no pages");
        assert_eq!(
            Error::Parse("x".into()).to_string(),
            "cannot parse the PDF: x"
        );
    }
}
