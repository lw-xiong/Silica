//! Mermaid diagram rendering: parses Mermaid markdown fences and renders
//! them to textures for GPUI via pure-Rust Mermaid layout and resvg rasterization.

use gpui::RenderImage;
use image::{Frame, RgbaImage};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

/// Cached rendered Mermaid diagram textures.
type Cache = HashMap<(String, bool), Result<Arc<RenderImage>, String>>;

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(HashMap::new());
}

/// Render Mermaid diagram source to a GPU-ready [`RenderImage`].
pub fn render_mermaid(source: &str, dark: bool) -> Result<Arc<RenderImage>, String> {
    let key = (source.trim().to_string(), dark);
    if let Some(hit) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return hit;
    }

    let result = render_uncached(&key.0, dark);

    CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        if cache.len() > 512 {
            cache.clear();
        }
        cache.insert(key, result.clone());
    });

    result
}

fn render_uncached(source: &str, dark: bool) -> Result<Arc<RenderImage>, String> {
    let theme = if dark {
        mermaid_rs_renderer::Theme::dark()
    } else {
        mermaid_rs_renderer::Theme::modern()
    };
    let opts = mermaid_rs_renderer::RenderOptions {
        theme,
        layout: mermaid_rs_renderer::LayoutConfig::default(),
    };

    let svg = mermaid_rs_renderer::render_with_options(source, opts)
        .map_err(|e| format!("{e}"))?;

    let mut opt = usvg::Options::default();
    opt.fontdb_mut().load_system_fonts();
    let tree = usvg::Tree::from_str(&svg, &opt).map_err(|e| format!("{e}"))?;

    let size = tree.size();
    let scale = 2.0f32; // 2x scale for sharp HiDPI rendering
    let w = (size.width() * scale).ceil().max(1.0) as u32;
    let h = (size.height() * scale).ceil().max(1.0) as u32;

    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)
        .ok_or_else(|| "Failed to allocate pixmap".to_string())?;

    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    let mut rgba = pixmap.take();
    // GPUI expects BGRA8 textures (swap red and blue)
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
        assert!(image.as_bytes(0).map_or(false, |b| !b.is_empty()));
    }

    #[test]
    fn catches_invalid_syntax() {
        let bad = "not a valid mermaid diagram at all %%%";
        let err = render_mermaid(bad, false);
        assert!(err.is_err());
    }
}
