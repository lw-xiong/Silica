//! The documents behind every PDF drawing: a cache of parsed files and of
//! rendered pages, shared by the open view, the embeds and the canvas.
//!
//! A `.pdf` is parsed once ([`Documents::document`]) and a page is rendered
//! once per scale ([`Documents::page`]); both happen on the background
//! executor and both answer "not yet" ([`DocState::Loading`], `None`) while
//! the work runs, then refresh every window so the drawing that asked comes
//! back for the result. A rendered page becomes a gpui [`RenderImage`], which
//! is the texture the sprite atlas keeps, so evicting one has to tell gpui
//! too ([`gpui::App::drop_image`]); the cache is bounded by
//! [`PAGE_CACHE_BYTES`] and evicts the pages least recently asked for.
//!
//! The scale a page is rendered at is pixels per point — the zoom the view
//! shows it at times the window's scale factor — so a page on a 2x display
//! is rasterized at 2x and drawn at its logical size, as text is. The caller
//! sizes the `img` explicitly; a [`RenderImage`] made here carries no scale
//! factor of its own.

use gpui::{App, Global, ImageSource, RenderImage};
use image::{Frame, RgbaImage};
use silica_pdf::{Bitmap, Document, OutlineItem, PageSize};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

/// How many bytes of rendered pages are kept before the least recently used
/// are let go. An A4 page at 2x is about 8 MB; this is a few dozen of them.
pub const PAGE_CACHE_BYTES: usize = 256 << 20;

/// A parsed file and what every drawing reads off it before any page is
/// rendered: the page sizes for layout, the outline for the sidebar, the
/// title.
pub struct Loaded {
    pub document: Document,
    /// Every page's size in points, as displayed (rotation applied).
    pub sizes: Vec<PageSize>,
    pub outline: Vec<OutlineItem>,
    pub title: Option<String>,
}

/// Where a file is between being asked for and being drawable.
#[derive(Clone)]
pub enum DocState {
    /// Being read and parsed; ask again after the next refresh.
    Loading,
    Ready(Arc<Loaded>),
    /// Could not be read or parsed: the message to show.
    Failed(String),
}

/// The cache. One per app, reached through [`documents`]; cheap to clone,
/// every clone is the same cache.
#[derive(Clone, Default)]
pub struct Documents {
    inner: Arc<Mutex<Inner>>,
}

impl Global for Documents {}

/// The app's cache, created on first use.
pub fn documents(cx: &mut App) -> Documents {
    cx.default_global::<Documents>().clone()
}

#[derive(Default)]
struct Inner {
    docs: HashMap<PathBuf, DocState>,
    pages: HashMap<PageKey, PageState>,
    /// Ready pages, least recently used first.
    order: VecDeque<PageKey>,
    /// Bytes of the ready pages.
    bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PageKey {
    path: PathBuf,
    page: usize,
    /// Pixels per point, in thousandths, so a key is hashable and two asks
    /// for the same zoom share a render.
    scale: u32,
}

impl PageKey {
    fn new(path: &Path, page: usize, scale: f32) -> PageKey {
        PageKey {
            path: path.to_path_buf(),
            page,
            scale: ((scale.max(0.001)) * 1000.0).round() as u32,
        }
    }

    fn scale(&self) -> f32 {
        self.scale as f32 / 1000.0
    }
}

enum PageState {
    Rendering,
    Ready(Arc<RenderImage>),
}

impl Inner {
    fn lock(inner: &Arc<Mutex<Inner>>) -> std::sync::MutexGuard<'_, Inner> {
        inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Moves `key` to the most recently used end.
    fn touch(&mut self, key: &PageKey) {
        if let Some(ix) = self.order.iter().position(|k| k == key) {
            self.order.remove(ix);
        }
        self.order.push_back(key.clone());
    }

    /// Records a rendered page and returns the images that had to go to
    /// make room, for the caller to hand back to gpui.
    fn insert_ready(&mut self, key: PageKey, image: Arc<RenderImage>) -> Vec<Arc<RenderImage>> {
        self.bytes += image_bytes(&image);
        self.pages.insert(key.clone(), PageState::Ready(image));
        self.touch(&key);
        self.evict(PAGE_CACHE_BYTES, Some(&key))
    }

    /// Lets go of the least recently used pages until `budget` is met,
    /// keeping `except` whatever its size (a page too big for the whole
    /// budget is still the page that was asked for).
    fn evict(&mut self, budget: usize, except: Option<&PageKey>) -> Vec<Arc<RenderImage>> {
        let mut gone = Vec::new();
        while self.bytes > budget {
            let Some(ix) = self.order.iter().position(|k| Some(k) != except) else {
                break;
            };
            let Some(key) = self.order.remove(ix) else {
                break;
            };
            if let Some(PageState::Ready(image)) = self.pages.remove(&key) {
                self.bytes = self.bytes.saturating_sub(image_bytes(&image));
                gone.push(image);
            }
        }
        gone
    }

    /// Forgets a file and every page rendered from it.
    fn forget(&mut self, path: &Path) -> Vec<Arc<RenderImage>> {
        self.docs.remove(path);
        let keys: Vec<PageKey> = self
            .pages
            .keys()
            .filter(|k| k.path == path)
            .cloned()
            .collect();
        let mut gone = Vec::new();
        for key in keys {
            self.order.retain(|k| *k != key);
            if let Some(PageState::Ready(image)) = self.pages.remove(&key) {
                self.bytes = self.bytes.saturating_sub(image_bytes(&image));
                gone.push(image);
            }
        }
        gone
    }
}

fn image_bytes(image: &RenderImage) -> usize {
    let size = image.size(0);
    (i32::from(size.width).max(0) as usize) * (i32::from(size.height).max(0) as usize) * 4
}

impl Documents {
    /// The file at `absolute`: parsed, being parsed, or why it could not be.
    /// The first ask starts the read on the background executor and refreshes
    /// the windows when it lands.
    pub fn document(&self, absolute: &Path, cx: &mut App) -> DocState {
        {
            let mut inner = Inner::lock(&self.inner);
            if let Some(state) = inner.docs.get(absolute) {
                return state.clone();
            }
            inner.docs.insert(absolute.to_path_buf(), DocState::Loading);
        }
        let this = self.clone();
        let path = absolute.to_path_buf();
        cx.spawn(async move |cx| {
            let read = path.clone();
            let state = cx
                .background_executor()
                .spawn(async move { load(&read) })
                .await;
            Inner::lock(&this.inner).docs.insert(path, state);
            cx.update(|cx| cx.refresh_windows());
        })
        .detach();
        DocState::Loading
    }

    /// Page `page` of the file at `absolute`, rendered at `scale` pixels per
    /// point, when it has been; the first ask starts the render (and the
    /// parse, if the file has not been asked for) and refreshes the windows
    /// when it lands.
    pub fn page(
        &self,
        absolute: &Path,
        page: usize,
        scale: f32,
        cx: &mut App,
    ) -> Option<Arc<RenderImage>> {
        let DocState::Ready(loaded) = self.document(absolute, cx) else {
            return None;
        };
        if page >= loaded.sizes.len() {
            return None;
        }
        let key = PageKey::new(absolute, page, scale);
        {
            let mut inner = Inner::lock(&self.inner);
            match inner.pages.get(&key) {
                Some(PageState::Ready(image)) => {
                    let image = image.clone();
                    inner.touch(&key);
                    return Some(image);
                }
                Some(PageState::Rendering) => return None,
                None => {}
            }
            inner.pages.insert(key.clone(), PageState::Rendering);
        }
        let this = self.clone();
        cx.spawn(async move |cx| {
            let (doc, index, scale) = (loaded.clone(), key.page, key.scale());
            let image = cx
                .background_executor()
                .spawn(async move { doc.document.render(index, scale).and_then(render_image) })
                .await;
            let gone = {
                let mut inner = Inner::lock(&this.inner);
                match image {
                    Some(image) => inner.insert_ready(key, image),
                    None => {
                        // A page that will not render is asked for again
                        // next frame otherwise, forever.
                        inner.pages.remove(&key);
                        Vec::new()
                    }
                }
            };
            cx.update(|cx| {
                for image in gone {
                    cx.drop_image(image, None);
                }
                cx.refresh_windows();
            });
        })
        .detach();
        None
    }

    /// Whether page `page` at `scale` is rendered and held, without asking
    /// for it: what a view counts for the socket.
    pub fn has_page(&self, absolute: &Path, page: usize, scale: f32) -> bool {
        matches!(
            Inner::lock(&self.inner)
                .pages
                .get(&PageKey::new(absolute, page, scale)),
            Some(PageState::Ready(_))
        )
    }

    /// The file changed or went away: forget it and every page rendered
    /// from it, so the next ask reads it afresh.
    pub fn invalidate(&self, absolute: &Path, cx: &mut App) {
        let gone = Inner::lock(&self.inner).forget(absolute);
        for image in gone {
            cx.drop_image(image, None);
        }
        cx.refresh_windows();
    }

    /// Bytes of rendered pages held, for the socket's state dump and tests.
    pub fn cached_bytes(&self) -> usize {
        Inner::lock(&self.inner).bytes
    }
}

/// An [`ImageSource`] for `img(..)` that draws page `page` at `scale`
/// pixels per point once it is rendered, and nothing until then.
pub fn page_source(absolute: PathBuf, page: usize, scale: f32) -> ImageSource {
    ImageSource::Custom(Arc::new(move |_window, cx| {
        documents(cx).page(&absolute, page, scale, cx).map(Ok)
    }))
}

fn load(path: &Path) -> DocState {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) => return DocState::Failed(err.to_string()),
    };
    match Document::open(bytes) {
        Ok(document) => {
            let sizes = document.sizes();
            let outline = document.outline();
            let title = document.title();
            DocState::Ready(Arc::new(Loaded {
                document,
                sizes,
                outline,
                title,
            }))
        }
        Err(err) => DocState::Failed(err.to_string()),
    }
}

/// A rendered page as the texture gpui keeps: its `RenderImage` is BGRA
/// (that is what every `img` decoder writes into one), straight alpha, and
/// a page is opaque, so the swap is the whole conversion.
fn render_image(bitmap: Bitmap) -> Option<Arc<RenderImage>> {
    let Bitmap {
        width,
        height,
        mut rgba,
    } = bitmap;
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let buffer = RgbaImage::from_raw(width, height, rgba)?;
    Some(Arc::new(RenderImage::new(vec![Frame::new(buffer)])))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(w: u32, h: u32) -> Arc<RenderImage> {
        render_image(Bitmap {
            width: w,
            height: h,
            rgba: vec![0; (w * h * 4) as usize],
        })
        .unwrap()
    }

    #[test]
    fn a_rendered_page_is_swapped_to_bgra() {
        let image = render_image(Bitmap {
            width: 1,
            height: 1,
            rgba: vec![10, 20, 30, 255],
        })
        .unwrap();
        assert_eq!(image.as_bytes(0), Some(&[30, 20, 10, 255][..]));
        assert_eq!(image_bytes(&image), 4);
    }

    #[test]
    fn a_short_buffer_renders_nothing() {
        assert!(render_image(Bitmap {
            width: 2,
            height: 2,
            rgba: vec![0; 4],
        })
        .is_none());
    }

    #[test]
    fn the_cache_evicts_least_recently_used_pages_first() {
        let mut inner = Inner::default();
        let path = Path::new("/v/a.pdf");
        let k = |p| PageKey::new(path, p, 1.0);
        for p in 0..4 {
            assert!(inner.insert_ready(k(p), image(10, 10)).is_empty());
        }
        assert_eq!(inner.bytes, 1600);
        // Page 0 is touched, so page 1 is the oldest.
        inner.touch(&k(0));
        let gone = inner.evict(1000, None);
        assert_eq!(gone.len(), 2);
        assert!(inner.pages.contains_key(&k(0)));
        assert!(!inner.pages.contains_key(&k(1)));
        assert!(!inner.pages.contains_key(&k(2)));
        assert!(inner.pages.contains_key(&k(3)));
        assert_eq!(inner.bytes, 800);
    }

    #[test]
    fn the_page_just_rendered_survives_a_budget_it_alone_exceeds() {
        let mut inner = Inner::default();
        let path = Path::new("/v/a.pdf");
        let small = PageKey::new(path, 0, 1.0);
        inner.insert_ready(small.clone(), image(2, 2));
        let big = PageKey::new(path, 1, 1.0);
        inner.bytes = PAGE_CACHE_BYTES; // pretend the cache is full already
        let gone = inner.insert_ready(big.clone(), image(4, 4));
        assert_eq!(gone.len(), 1);
        assert!(inner.pages.contains_key(&big));
        assert!(!inner.pages.contains_key(&small));
    }

    #[test]
    fn forgetting_a_file_drops_its_pages_and_no_others() {
        let mut inner = Inner::default();
        let a = Path::new("/v/a.pdf");
        let b = Path::new("/v/b.pdf");
        inner.docs.insert(a.to_path_buf(), DocState::Loading);
        inner.insert_ready(PageKey::new(a, 0, 1.0), image(1, 1));
        inner.insert_ready(PageKey::new(a, 1, 2.0), image(1, 1));
        inner.insert_ready(PageKey::new(b, 0, 1.0), image(1, 1));
        let gone = inner.forget(a);
        assert_eq!(gone.len(), 2);
        assert!(inner.docs.is_empty());
        assert_eq!(inner.pages.len(), 1);
        assert_eq!(inner.order.len(), 1);
        assert_eq!(inner.bytes, 4);
    }

    #[test]
    fn a_key_rounds_the_scale_to_thousandths() {
        let path = Path::new("/v/a.pdf");
        assert_eq!(PageKey::new(path, 0, 1.0004), PageKey::new(path, 0, 1.0));
        assert_ne!(PageKey::new(path, 0, 1.001), PageKey::new(path, 0, 1.0));
        assert_eq!(PageKey::new(path, 0, 0.0).scale(), 0.001);
    }
}
