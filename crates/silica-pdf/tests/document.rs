//! The crate over the committed fixtures: `pages.pdf` (typst, five pages, an
//! outline, a landscape third page), `rotated.pdf` and `blank.pdf` (hand-written;
//! see `fixtures/README.md`).

use std::time::Instant;

use silica_pdf::{Bitmap, Document, Error, OutlineItem, PageSize};

const A4_WIDTH: f32 = 595.276;
const A4_HEIGHT: f32 = 841.89;

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"))
}

fn open(name: &str) -> Document {
    Document::open(fixture(name)).unwrap_or_else(|e| panic!("opening {name}: {e}"))
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

fn assert_size(size: PageSize, width: f32, height: f32) {
    assert!(
        close(size.width, width) && close(size.height, height),
        "expected {width} x {height}, got {} x {}",
        size.width,
        size.height
    );
}

/// Pixels darker than mid grey in every channel.
fn dark_pixels(bitmap: &Bitmap) -> usize {
    bitmap
        .rgba
        .chunks_exact(4)
        .filter(|p| p[0] < 128 && p[1] < 128 && p[2] < 128)
        .count()
}

fn pixel(bitmap: &Bitmap, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * bitmap.width + x) * 4) as usize;
    bitmap.rgba[at..at + 4].try_into().unwrap()
}

#[test]
fn pages_fixture_has_five_pages_with_the_third_landscape() {
    let doc = open("pages.pdf");
    assert_eq!(doc.page_count(), 5);
    let sizes = doc.sizes();
    assert_eq!(sizes.len(), 5);
    for (index, size) in sizes.iter().enumerate() {
        if index == 2 {
            assert_size(*size, A4_HEIGHT, A4_WIDTH);
        } else {
            assert_size(*size, A4_WIDTH, A4_HEIGHT);
        }
        assert_eq!(doc.page_size(index), Some(*size));
    }
    assert_eq!(doc.page_size(5), None);
}

#[test]
fn pages_fixture_title() {
    assert_eq!(open("pages.pdf").title().as_deref(), Some("Pages fixture"));
}

#[test]
fn hand_written_files_have_no_title() {
    assert_eq!(open("blank.pdf").title(), None);
    assert_eq!(open("rotated.pdf").title(), None);
}

#[test]
fn pages_fixture_outline() {
    let item = |title: &str, page: usize, depth: usize| OutlineItem {
        title: title.to_string(),
        page: Some(page),
        depth,
    };
    assert_eq!(
        open("pages.pdf").outline(),
        vec![
            item("One", 0, 0),
            item("One point one", 1, 1),
            item("Two", 2, 0),
            item("Two point one", 3, 1),
            item("Three", 4, 0),
        ]
    );
}

#[test]
fn hand_written_files_have_no_outline() {
    assert!(open("blank.pdf").outline().is_empty());
    assert!(open("rotated.pdf").outline().is_empty());
}

#[test]
fn pages_fixture_text() {
    let doc = open("pages.pdf");
    let expected = [
        "This is page one.",
        "This is page two.",
        "This is page three.",
        "This is page four.",
        "This is page five.",
    ];
    for (index, sentence) in expected.iter().enumerate() {
        let text = doc.page_text(index).unwrap();
        assert!(
            text.contains(sentence),
            "page {index} text {text:?} lacks {sentence:?}"
        );
    }
    let page_two = doc.page_text(2).unwrap();
    assert!(page_two.starts_with("Two\n"), "{page_two:?}");
    assert_eq!(doc.page_text(5), None);
}

#[test]
fn blank_page_has_empty_text() {
    assert_eq!(open("blank.pdf").page_text(0).as_deref(), Some(""));
}

#[test]
fn pages_fixture_renders_at_two_scales() {
    let doc = open("pages.pdf");
    let one = doc.render(0, 1.0).unwrap();
    let two = doc.render(0, 2.0).unwrap();
    assert_eq!(
        (one.width, one.height),
        (A4_WIDTH.ceil() as u32, A4_HEIGHT.ceil() as u32)
    );
    assert_eq!(
        (two.width, two.height),
        (
            (A4_WIDTH * 2.0).ceil() as u32,
            (A4_HEIGHT * 2.0).ceil() as u32
        )
    );
    for bitmap in [&one, &two] {
        assert_eq!(
            bitmap.rgba.len(),
            (bitmap.width * bitmap.height * 4) as usize
        );
        assert!(bitmap.rgba.chunks_exact(4).all(|p| p[3] == 255));
        let white = bitmap
            .rgba
            .chunks_exact(4)
            .filter(|p| p[0] == 255 && p[1] == 255 && p[2] == 255)
            .count();
        let total = (bitmap.width * bitmap.height) as usize;
        assert!(
            white * 10 > total * 9,
            "only {white} of {total} pixels are white"
        );
    }
    let dark_one = dark_pixels(&one);
    let dark_two = dark_pixels(&two);
    assert!(
        dark_one > 50,
        "page 0 at 1x has only {dark_one} dark pixels"
    );
    let ratio = dark_two as f64 / dark_one as f64;
    assert!(
        (3.0..5.0).contains(&ratio),
        "2x/1x dark ratio {ratio} ({dark_two}/{dark_one})"
    );
}

#[test]
fn render_rejects_bad_inputs() {
    let doc = open("pages.pdf");
    assert!(doc.render(5, 1.0).is_none());
    assert!(doc.render(0, 0.0).is_none());
    assert!(doc.render(0, -1.0).is_none());
    assert!(doc.render(0, f32::NAN).is_none());
    assert!(doc.render(0, f32::INFINITY).is_none());
}

#[test]
fn render_scales_down_to_the_maximum_side() {
    let bitmap = open("pages.pdf").render(0, 100.0).unwrap();
    assert_eq!(bitmap.height, silica_pdf::MAX_BITMAP_SIDE);
    assert!(bitmap.width < bitmap.height);
    assert_eq!(
        bitmap.rgba.len(),
        (bitmap.width * bitmap.height * 4) as usize
    );
}

#[test]
fn rotated_page_is_reported_and_drawn_rotated() {
    let doc = open("rotated.pdf");
    assert_eq!(doc.page_count(), 1);
    assert_size(doc.page_size(0).unwrap(), 100.0, 200.0);
    let bitmap = doc.render(0, 1.0).unwrap();
    assert_eq!((bitmap.width, bitmap.height), (100, 200));
    // The unrotated page's bottom-left quadrant, turned clockwise, is the
    // displayed page's top-left quarter.
    assert_eq!(pixel(&bitmap, 25, 50), [0, 0, 0, 255]);
    assert_eq!(pixel(&bitmap, 10, 10), [0, 0, 0, 255]);
    assert_eq!(pixel(&bitmap, 40, 90), [0, 0, 0, 255]);
    assert_eq!(pixel(&bitmap, 75, 50), [255, 255, 255, 255]);
    assert_eq!(pixel(&bitmap, 25, 150), [255, 255, 255, 255]);
    assert_eq!(pixel(&bitmap, 75, 150), [255, 255, 255, 255]);
    let dark = dark_pixels(&bitmap);
    assert!(
        (4800..=5200).contains(&dark),
        "{dark} dark pixels, expected about 5000"
    );
    let twice = doc.render(0, 2.0).unwrap();
    assert_eq!((twice.width, twice.height), (200, 400));
    assert_eq!(pixel(&twice, 50, 100), [0, 0, 0, 255]);
    assert_eq!(pixel(&twice, 150, 300), [255, 255, 255, 255]);
}

#[test]
fn blank_page_renders_white() {
    let doc = open("blank.pdf");
    assert_size(doc.page_size(0).unwrap(), A4_WIDTH, A4_HEIGHT);
    let bitmap = doc.render(0, 0.25).unwrap();
    assert_eq!((bitmap.width, bitmap.height), (149, 211));
    assert!(bitmap
        .rgba
        .chunks_exact(4)
        .all(|p| p == [255, 255, 255, 255]));
}

#[test]
fn garbage_is_an_error_not_a_panic() {
    assert!(matches!(Document::open(Vec::new()), Err(Error::Parse(_))));
    assert!(matches!(
        Document::open(b"hello".to_vec()),
        Err(Error::Parse(_))
    ));
    assert!(matches!(
        Document::open(b"%PDF-1.7\n".to_vec()),
        Err(Error::Parse(_))
    ));
    let noise: Vec<u8> = (0..20_000u32)
        .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)
        .collect();
    assert!(matches!(Document::open(noise), Err(Error::Parse(_))));
    let pages = fixture("pages.pdf");
    for cut in [10, 100, 1000, pages.len() / 2, pages.len() - 30] {
        let truncated = pages[..cut].to_vec();
        match Document::open(truncated) {
            // hayro repairs some truncations by scanning for objects; either
            // outcome is fine as long as nothing panics and the pages read.
            Ok(doc) => {
                let _ = doc.sizes();
                let _ = doc.outline();
                let _ = doc.title();
                for index in 0..doc.page_count() {
                    let _ = doc.render(index, 0.5);
                    let _ = doc.page_text(index);
                }
            }
            Err(Error::Parse(_)) | Err(Error::Empty) => {}
            Err(Error::Encrypted) => panic!("a truncated file is not encrypted"),
        }
    }
}

#[test]
fn page_tree_without_pages_is_empty_or_parse() {
    let bytes = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [] /Count 0 >>\nendobj\ntrailer\n<< /Root 1 0 R >>\n".to_vec();
    assert!(matches!(
        Document::open(bytes),
        Err(Error::Empty) | Err(Error::Parse(_))
    ));
}

#[test]
fn document_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Document>();
    assert_send_sync::<Bitmap>();
    assert_send_sync::<OutlineItem>();
    assert_send_sync::<Error>();
}

#[test]
fn renders_from_another_thread_while_sizes_are_read() {
    let doc = std::sync::Arc::new(open("pages.pdf"));
    let worker = {
        let doc = doc.clone();
        std::thread::spawn(move || doc.render(2, 0.5).unwrap())
    };
    let sizes = doc.sizes();
    let bitmap = worker.join().unwrap();
    assert_eq!(bitmap.width, (sizes[2].width * 0.5).ceil() as u32);
}

/// `cargo test -p silica-pdf -- --ignored --nocapture render_timings` prints what
/// a page costs; the view's author sizes its render queue on these numbers.
#[test]
#[ignore]
fn render_timings() {
    let doc = open("pages.pdf");
    let started = Instant::now();
    let doc_text: Vec<_> = (0..doc.page_count()).map(|i| doc.page_text(i)).collect();
    println!(
        "page_text for {} pages: {:?}",
        doc_text.len(),
        started.elapsed()
    );
    for (index, text) in doc_text.iter().enumerate() {
        println!("page {index} text: {text:?}");
    }
    for scale in [1.0f32, 2.0] {
        for index in 0..doc.page_count() {
            let started = Instant::now();
            let bitmap = doc.render(index, scale).unwrap();
            let elapsed = started.elapsed();
            println!(
                "page {index} at {scale}x: {} x {} px, {} KB, {elapsed:?}",
                bitmap.width,
                bitmap.height,
                bitmap.rgba.len() / 1024
            );
        }
    }
    let started = Instant::now();
    let bitmap = doc.render(0, 2.0).unwrap();
    println!(
        "page 0 at 2x again: {} x {} px in {:?}",
        bitmap.width,
        bitmap.height,
        started.elapsed()
    );
}
