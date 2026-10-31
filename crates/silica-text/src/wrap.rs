//! Which visual row a byte sits on when a line is soft-wrapped.
//!
//! The shaping itself belongs to the view, but the rule for a byte that
//! falls exactly on a wrap point does not: the caret there is at the start
//! of the later row, not past the end of the earlier one, and Home, End,
//! Up, Down, the selection highlight and the painted caret all have to
//! agree about that or the keys contradict what the screen shows.

/// The visual row of display byte `index`, given the display byte each
/// wrapped row after the first begins at, in order. A byte on a wrap point
/// belongs to the row it begins.
pub fn row_of(row_starts: impl IntoIterator<Item = usize>, index: usize) -> usize {
    row_starts
        .into_iter()
        .take_while(|&start| start <= index)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "the quick brown" wrapped into "the ", "quick ", "brown": the rows
    /// start at 0, 4 and 10.
    #[test]
    fn a_byte_on_a_wrap_point_starts_the_later_row() {
        let starts = || [4, 10];
        assert_eq!(row_of(starts(), 0), 0);
        assert_eq!(row_of(starts(), 3), 0);
        // The wrap points themselves: the first byte of rows 1 and 2.
        assert_eq!(row_of(starts(), 4), 1);
        assert_eq!(row_of(starts(), 10), 2);
        assert_eq!(row_of(starts(), 7), 1);
        assert_eq!(row_of(starts(), 15), 2);
    }

    #[test]
    fn an_unwrapped_line_is_all_row_zero() {
        let unwrapped: [usize; 0] = [];
        assert_eq!(row_of(unwrapped, 0), 0);
        assert_eq!(row_of(unwrapped, 99), 0);
    }
}
