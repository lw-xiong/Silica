//! Mermaid diagram rendering: parses Mermaid markdown fences and renders
//! them to textures for GPUI via pure-Rust Mermaid layout and resvg rasterization.
//!
//! Warna diagram diambil dari token tema Silica (`crate::theme::Theme`,
//! gelap dan terang) — bukan ditulis ulang di sini — sehingga diagram ikut
//! berubah bersama tema vault. Render berskala: skala default 2x untuk
//! HiDPI tajam, dan [`render_mermaid_at`] render pada skala lebih tinggi
//! agar diagram yang diperbesar (zoom) tetap tajam; geser (pan) diurus wadah
//! scroll di sisi tampilan.

use crate::theme::Theme as AppTheme;
use gpui::RenderImage;
use image::{Frame, RgbaImage};
use std::cell::RefCell;
use std::num::NonZeroUsize;
use std::sync::Arc;

/// How many rendered diagrams are kept.
///
/// A vault is edited in one window, so the diagrams on screen are a handful;
/// 256 leaves room for a long note scrolled back and forth. Past that the
/// oldest *unused* entry goes, not the whole cache — clearing it threw away the
/// diagrams still being painted, so every eviction cost a re-render of
/// everything visible.
const CAPACITY: NonZeroUsize = match NonZeroUsize::new(256) {
    Some(n) => n,
    // * Unreachable for 256, and `MIN` keeps this a `const` with no panic
    // * path in the binary.
    None => NonZeroUsize::MIN,
};

/// Cached rendered Mermaid diagram textures, least-recently-used first out.
type Cache = lru::LruCache<(String, bool, u32), Result<Arc<RenderImage>, String>>;

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache::new(CAPACITY));
}

/// 2x skala render default (HiDPI tajam).
const DEFAULT_SCALE: f32 = 2.0;
/// Sisi panjang maksimum render agar diagram sangat besar tidak mengalokasi
/// pixmap raksasa dan tetap responsif.
const MAX_SIDE: f32 = 4096.0;

/// Render Mermaid diagram source to a GPU-ready [`RenderImage`].
pub fn render_mermaid(source: &str, dark: bool) -> Result<Arc<RenderImage>, String> {
    render_mermaid_at(source, dark, DEFAULT_SCALE)
}

/// Render `source` pada skala `scale`: skala lebih tinggi menghasilkan
/// bitmap lebih detail, cocok untuk diagram yang di-zoom. `pan` (geser)
/// ditangani wadah scroll di sisi tampilan, bukan di sini.
pub fn render_mermaid_at(source: &str, dark: bool, scale: f32) -> Result<Arc<RenderImage>, String> {
    let key = (
        source.trim().to_string(),
        dark,
        // * `to_bits` beda skala = beda kunci, tanpa cast float ke int.
        scale.to_bits(),
    );
    // * `get` moves the entry to the front, so what is on screen stays; only
    // * what nobody has asked for is evicted.
    if let Some(hit) = CACHE.with(|c| c.borrow_mut().get(&key).cloned()) {
        return hit;
    }

    let result = render_uncached(&key.0, dark, scale);

    CACHE.with(|c| {
        c.borrow_mut().put(key, result.clone());
    });

    result
}

/// Bangun [`mermaid_rs_renderer::Theme`] dari token tema Silica yang ada,
/// jangan hardcode warna baru di sini.
fn mermaid_theme(dark: bool) -> mermaid_rs_renderer::Theme {
    let t = if dark {
        AppTheme::dark()
    } else {
        AppTheme::light()
    };
    let mut theme = if dark {
        mermaid_rs_renderer::Theme::dark()
    } else {
        mermaid_rs_renderer::Theme::modern()
    };
    let bg = t.bg.to_string();
    let border = t.border.to_string();
    let panel = t.bg_secondary.to_string();
    let text = t.text.to_string();
    theme.font_family = t.text_font.as_str().to_string();
    theme.font_size = t.base_font_size;
    theme.background = bg.clone();
    theme.text_color = text.clone();
    theme.primary_text_color = text;
    theme.primary_border_color = border.clone();
    theme.line_color = border.clone();
    theme.primary_color = panel.clone();
    theme.secondary_color = panel.clone();
    theme.tertiary_color = bg;
    theme.cluster_background = panel.clone();
    theme.cluster_border = border.clone();
    theme.edge_label_background = panel.clone();
    theme.sequence_actor_fill = panel.clone();
    theme.sequence_actor_border = border.clone();
    theme.sequence_actor_line = border.clone();
    theme.sequence_note_fill = panel.clone();
    theme.sequence_note_border = border.clone();
    theme.sequence_activation_fill = panel;
    theme.sequence_activation_border = border;
    theme
}

fn render_uncached(source: &str, dark: bool, scale: f32) -> Result<Arc<RenderImage>, String> {
    let theme = mermaid_theme(dark);
    let opts = mermaid_rs_renderer::RenderOptions {
        theme,
        layout: mermaid_rs_renderer::LayoutConfig::default(),
    };

    let svg = mermaid_rs_renderer::render_with_options(source, opts).map_err(|e| format!("{e}"))?;

    let mut opt = usvg::Options::default();
    opt.fontdb_mut().load_system_fonts();
    let tree = usvg::Tree::from_str(&svg, &opt).map_err(|e| format!("{e}"))?;

    let size = tree.size();
    let mut w = (size.width() * scale).ceil().max(1.0);
    let mut h = (size.height() * scale).ceil().max(1.0);
    if w > MAX_SIDE || h > MAX_SIDE {
        let ratio = MAX_SIDE / w.max(h);
        w *= ratio;
        h *= ratio;
    }
    // * `try_into` tidak tersedia untuk `f32`; ukuran sudah terbatas dan
    // * non-negatif di atas, cast ke u32 aman.
    #[allow(clippy::cast_possible_truncation)]
    let w = w as u32;
    #[allow(clippy::cast_possible_truncation)]
    let h = h as u32;

    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)
        .ok_or_else(|| "Failed to allocate pixmap".to_string())?;

    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    let mut rgba = pixmap.take();
    // * GPUI expects BGRA8 textures (swap red and blue)
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }

    let buffer = RgbaImage::from_raw(w, h, rgba)
        .ok_or_else(|| "Failed to construct RgbaImage".to_string())?;

    Ok(Arc::new(RenderImage::new(vec![Frame::new(buffer)])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_simple_flowchart() {
        let diagram = "flowchart LR\n    A-->B";
        let image = render_mermaid(diagram, false).expect("flowchart rendered");
        assert!(image.as_bytes(0).is_some_and(|b| !b.is_empty()));
    }

    #[test]
    fn catches_invalid_syntax() {
        let bad = "not a valid mermaid diagram at all %%%";
        let err = render_mermaid(bad, false);
        assert!(err.is_err());
    }

    #[test]
    fn light_and_dark_themes_produce_different_output() {
        let diagram = "flowchart LR\n    A-->B";
        let light = render_mermaid(diagram, false).expect("light rendered");
        let dark = render_mermaid(diagram, true).expect("dark rendered");
        let light = light.as_bytes(0).expect("light bytes");
        let dark = dark.as_bytes(0).expect("dark bytes");
        assert_ne!(
            light, dark,
            "tema gelap dan terang harus menghasilkan warna berbeda"
        );
    }

    #[test]
    fn zoom_scale_renders_larger_bitmap() {
        let diagram = "flowchart LR\n    A-->B";
        let small = render_mermaid_at(diagram, false, 1.0).expect("scale 1 rendered");
        let large = render_mermaid_at(diagram, false, 4.0).expect("scale 4 rendered");
        assert!(
            large.as_bytes(0).map(|b| b.len()).unwrap_or(0)
                > small.as_bytes(0).map(|b| b.len()).unwrap_or(0),
            "skala render lebih besar harus menghasilkan bitmap lebih besar"
        );
    }
}
