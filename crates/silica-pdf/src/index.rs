//! A full-text index over a PDF's pages.
//!
//! [`crate::Document::page_text`] gives one page at a time; a search over the
//! vault wants the whole document as one string with the page each byte
//! belongs to, so a hit can be reported as the page it sits on and the link
//! subpath that opens it. [`PdfIndex`] is that: every page's text in order,
//! joined by a newline so a page's words never run into its neighbour's.
//!
//! The index holds no glyphs and does no OCR: it is only the text the page
//! already draws, extracted by [`crate::Document::page_text`].

use std::ops::Range;

/// One document's page text as a single searchable string.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PdfIndex {
    /// Every page's text in order, each page followed by a newline except
    /// the last, so a term never straddles two pages by more than that
    /// separator.
    pub text: String,
    /// The range of each page's own text inside [`PdfIndex::text`], indexed
    /// by the page. A page the interpreter gave up on is an empty range, so
    /// the pages after it keep their numbers.
    pub pages: Vec<Range<usize>>,
}

impl PdfIndex {
    /// Builds the index from each page's text in order; a page the
    /// interpreter gave up on (`None`) is an empty range.
    pub fn from_pages(pages: Vec<Option<String>>) -> PdfIndex {
        let total = pages.len();
        let mut index = PdfIndex::default();
        for (page, text) in pages.into_iter().enumerate() {
            let start = index.text.len();
            if let Some(text) = text {
                index.text.push_str(&text);
            }
            index.pages.push(start..index.text.len());
            if page + 1 < total {
                index.text.push('\n');
            }
        }
        index
    }

    /// The number of pages indexed.
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// The text of one page, or `None` past the last page.
    pub fn page_text(&self, page: usize) -> Option<&str> {
        self.text.get(self.pages.get(page)?.clone())
    }
}

/// The link subpath that opens a zero-based page: `#page=N` in the one-based
/// numbering a link writes and [`crate::parse_subpath`] reads back.
pub fn page_subpath(page: usize) -> String {
    format!("#page={}", page + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ops::Range;

    /// The page's own slice of the joined text, as the ranges describe it.
    fn text<'a>(index: &'a PdfIndex, range: &Range<usize>) -> &'a str {
        &index.text[range.clone()]
    }

    fn index() -> PdfIndex {
        let pages: Vec<Option<String>> =
            vec![Some("alpha".to_string()), None, Some("gamma".to_string())];
        PdfIndex::from_pages(pages)
    }

    const TEXT: &str = "alpha\n\ngamma";

    #[test]
    fn pages_hold_their_own_text() {
        let index = index();
        assert_eq!(index.page_count(), 3);
        assert_eq!(index.text, TEXT);
        assert_eq!(text(&index, &index.pages[0]), "alpha");
        assert_eq!(text(&index, &index.pages[1]), "");
        assert_eq!(text(&index, &index.pages[2]), "gamma");
        assert_eq!(index.page_text(0), Some("alpha"));
        assert_eq!(index.page_text(1), Some(""));
        assert_eq!(index.page_text(2), Some("gamma"));
        assert_eq!(index.page_text(3), None);
    }

    #[test]
    fn ranges_are_in_order_and_cover_the_text() {
        let index = index();
        assert_eq!(index.pages[0], 0..5);
        assert_eq!(index.pages[1], 6..6);
        assert_eq!(index.pages[2], 7..12);
        for page in &index.pages {
            assert!(page.start <= page.end && page.end <= index.text.len());
        }
    }

    #[test]
    fn empty_document_indexes_to_nothing() {
        let index = PdfIndex::from_pages(Vec::new());
        assert_eq!(index.page_count(), 0);
        assert!(index.text.is_empty());
        assert!(index.pages.is_empty());
        assert_eq!(index.page_text(0), None);
    }

    #[test]
    fn one_page_takes_no_separator() {
        let index = PdfIndex::from_pages(vec![Some("solo".to_string())]);
        assert_eq!(index.text, "solo");
        assert_eq!(index.pages, vec![0..4]);
    }

    /// A page's text keeps whatever the extraction gave, including the
    /// newlines between its own lines.
    #[test]
    fn a_page_keeps_its_own_lines() {
        let index = PdfIndex::from_pages(vec![
            Some("first\nline".to_string()),
            Some("second".to_string()),
        ]);
        assert_eq!(index.pages, vec![0..10, 11..17]);
        assert_eq!(index.page_text(0), Some("first\nline"));
        assert_eq!(index.page_text(1), Some("second"));
    }

    #[test]
    fn the_locator_counts_pages_from_one() {
        assert_eq!(page_subpath(0), "#page=1");
        assert_eq!(page_subpath(4), "#page=5");
        // * The same one-based numbering a link writes, so the two agree.
        assert_eq!(crate::parse_subpath("#page=3").page, Some(2));
        assert_eq!(crate::parse_subpath(&page_subpath(2)).page, Some(2));
    }
}
