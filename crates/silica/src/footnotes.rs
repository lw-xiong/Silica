//! Rows for the right-dock Footnotes pane: one row per footnote in the
//! active note's reading model, in the order the Footnotes section lists
//! them. The parser is not touched: `Model.footnotes` already holds each
//! definition's source byte offset, and a row's click reveals that offset
//! with the workspace's existing `reveal_offset`.

use crate::reading::model::Model;

/// One footnote in the active note, as the Footnotes pane lists it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FootnoteRow {
    /// 1-based footnote number, as the reference shows it (`[1]`).
    pub n: usize,
    /// Source byte offset of the definition. Revealing it scrolls the
    /// reading view (or the editor, in an editing mode) to the definition.
    pub offset: usize,
}

/// The footnotes of `model`, numbered from 1 in section order.
pub fn footnote_rows(model: &Model) -> Vec<FootnoteRow> {
    model
        .footnotes
        .iter()
        .enumerate()
        .map(|(i, &offset)| FootnoteRow { n: i + 1, offset })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model_with(footnotes: Vec<usize>) -> Model {
        Model {
            blocks: Vec::new(),
            starts: Vec::new(),
            footnotes,
        }
    }

    #[test]
    fn footnote_rows_are_numbered_from_one_in_section_order() {
        let model = model_with(vec![10, 0, 25]);
        let rows = footnote_rows(&model);
        assert_eq!(
            rows,
            vec![
                FootnoteRow { n: 1, offset: 10 },
                FootnoteRow { n: 2, offset: 0 },
                FootnoteRow { n: 3, offset: 25 },
            ]
        );
    }

    #[test]
    fn no_footnotes_yields_no_rows() {
        assert!(footnote_rows(&model_with(Vec::new())).is_empty());
    }
}
