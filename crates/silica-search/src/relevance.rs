//! Ranking search results by relevance.
//!
//! A search pane lists every file a query matched, and the user picks the
//! order: Obsidian's six orders are name and time, while this module adds
//! the one a keyword search earns — the note the query matched most, and
//! earliest, and in its title, first. [`Relevance::of`] scores one result,
//! [`sort_by_relevance`] orders a list by that score, and [`sort`] keeps
//! every other order selectable alongside it.
//!
//! The score has three levels, in the order they matter:
//!
//! 1. **Points** — how often the query hit: a term appearing in three
//!    places says more about a note than the same term appearing once. A
//!    title hit weighs [`NAME_HIT_POINTS`], a hit in the text
//!    [`CONTENT_HIT_POINTS`]: the title is the strongest place a term can
//!    land, the way a `[key]` or `tag:` hit is the strongest place a
//!    property or tag match can.
//! 2. **Position** — where the first hit sits, on a scale of
//!    [`POSITION_SCALE`] that falls by one every [`POSITION_STEP`] bytes.
//!    A note that opens with the term is more about it than one that ends
//!    with it, and this is only ever a tie-break behind the count.
//! 3. **Recency** — the file's `mtime`, the last tie-break, so two notes
//!    matching identically keep the fresher one on top.
//!
//! [`Relevance`] orders itself so that *more relevant is greater*, so two
//! results can be compared without sorting a list. When the score ties
//! completely the name collation decides, then the path, so the order is
//! the list's content rather than the order the rows arrived in.

use crate::collate::natural_cmp;
use std::cmp::Reverse;
use std::ops::Range;

/// What the ranker needs from one result row. The pane's own result type
/// implements this; nothing here depends on a vault.
pub trait Ranked {
    /// The vault-relative path, the final tie-break.
    fn path(&self) -> &str;
    /// The base name shown as the result's title.
    fn name(&self) -> &str;
    /// Byte ranges of the query's hits in the note's text.
    fn content_hits(&self) -> &[Range<usize>];
    /// Byte ranges of the query's hits in the file name.
    fn name_hits(&self) -> &[Range<usize>];
    /// The file's modification time.
    fn mtime(&self) -> u64;
    /// The file's creation time.
    fn ctime(&self) -> u64;
}

/// Points for a hit in the note's text.
pub const CONTENT_HIT_POINTS: u64 = 2;
/// Points for a hit in the title: the strongest place a term can land.
pub const NAME_HIT_POINTS: u64 = 4;
/// The position of the first hit, on a scale where the top of the note
/// scores the full scale and each [`POSITION_STEP`] bytes deeper costs one.
pub const POSITION_SCALE: u32 = 1_000;
/// Bytes of depth per point of position.
pub const POSITION_STEP: usize = 8;

/// How relevant one result is: greater is more relevant. The fields are in
/// priority order, so the derived comparison compares points first, then
/// position, then recency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Relevance {
    /// Weighted hit count: [`NAME_HIT_POINTS`] per title hit plus
    /// [`CONTENT_HIT_POINTS`] per hit in the text.
    pub points: u64,
    /// How near the top of the note the first hit sits, 0 to
    /// [`POSITION_SCALE`]; a title-only match scores the full scale.
    pub position: u32,
    /// The file's mtime, the last tie-break.
    pub recency: u64,
}

impl Relevance {
    /// Scores one result.
    pub fn of<T: Ranked>(result: &T) -> Relevance {
        let content = result.content_hits().len() as u64;
        let name = result.name_hits().len() as u64;
        let first = result.content_hits().iter().map(|r| r.start).min();
        let position = match (first, name > 0) {
            (Some(at), _) => {
                let depth = (at / POSITION_STEP).min(POSITION_SCALE as usize) as u32;
                POSITION_SCALE.saturating_sub(depth)
            }
            (None, true) => POSITION_SCALE,
            (None, false) => 0,
        };
        Relevance {
            points: content * CONTENT_HIT_POINTS + name * NAME_HIT_POINTS,
            position,
            recency: result.mtime(),
        }
    }
}

/// The orders a result list can be shown in: the relevance order this
/// module adds, plus the name and time orders that were already there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResultSort {
    /// Relevance first: the most, earliest, title-most matches on top.
    #[default]
    Relevance,
    /// Name ascending, the pane's usual order.
    Alphabetical,
    /// Name descending.
    AlphabeticalReverse,
    /// Newest modified first.
    ByModifiedTime,
    /// Oldest modified first.
    ByModifiedTimeReverse,
    /// Newest created first.
    ByCreatedTime,
    /// Oldest created first.
    ByCreatedTimeReverse,
}

/// Orders `results` by relevance: by score, then by the name collation,
/// then by the path descending (a note in a folder sorts ahead of a root
/// note of the same name, the tie the other orders already use).
pub fn sort_by_relevance<T: Ranked>(results: &mut [T]) {
    results.sort_by(|a, b| {
        Relevance::of(b)
            .cmp(&Relevance::of(a))
            .then_with(|| natural_cmp(a.name(), b.name()))
            .then_with(|| b.path().cmp(a.path()))
    });
}

/// Orders `results` in `order`. The name and time orders are stable, so
/// rows that tie keep the order they had.
pub fn sort<T: Ranked>(results: &mut [T], order: ResultSort) {
    match order {
        ResultSort::Relevance => sort_by_relevance(results),
        ResultSort::Alphabetical => results.sort_by(|a, b| natural_cmp(a.name(), b.name())),
        ResultSort::AlphabeticalReverse => results.sort_by(|a, b| natural_cmp(b.name(), a.name())),
        ResultSort::ByModifiedTime => results.sort_by_key(|r| Reverse(r.mtime())),
        ResultSort::ByModifiedTimeReverse => results.sort_by_key(|r| r.mtime()),
        ResultSort::ByCreatedTime => results.sort_by_key(|r| Reverse(r.ctime())),
        ResultSort::ByCreatedTimeReverse => results.sort_by_key(|r| r.ctime()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ops::Range;

    /// One result row as the ranking sees it.
    struct Row {
        path: &'static str,
        name: &'static str,
        content: Vec<Range<usize>>,
        name_hits: Vec<Range<usize>>,
        mtime: u64,
        ctime: u64,
    }

    impl Ranked for Row {
        fn path(&self) -> &str {
            self.path
        }
        fn name(&self) -> &str {
            self.name
        }
        fn content_hits(&self) -> &[Range<usize>] {
            &self.content
        }
        fn name_hits(&self) -> &[Range<usize>] {
            &self.name_hits
        }
        fn mtime(&self) -> u64 {
            self.mtime
        }
        fn ctime(&self) -> u64 {
            self.ctime
        }
    }

    fn row(path: &'static str, content: &[usize], name_hits: &[usize]) -> Row {
        Row {
            path,
            name: path.rsplit('/').next().unwrap_or(path),
            content: content.iter().map(|&at| at..at + 3).collect(),
            name_hits: name_hits.iter().map(|&at| at..at + 3).collect(),
            mtime: 100,
            ctime: 100,
        }
    }

    fn names(results: &[Row]) -> Vec<&'static str> {
        results.iter().map(|r| r.path).collect()
    }

    /// The slice's acceptance test: a note the query matched three times
    /// ranks above a note it matched once, even though its name sorts last.
    #[test]
    fn three_matches_rank_above_one_match() {
        let mut results = vec![
            row("aaa.md", &[0], &[]),
            row("zzz.md", &[50, 200, 900], &[]),
        ];
        sort_by_relevance(&mut results);
        assert_eq!(names(&results), vec!["zzz.md", "aaa.md"]);
    }

    /// The count leads: three matches deep in a note outrank one at its top.
    #[test]
    fn the_count_outweighs_position() {
        let mut results = vec![
            row("early.md", &[0], &[]),
            row("many.md", &[900, 1500, 2500], &[]),
        ];
        sort_by_relevance(&mut results);
        assert_eq!(names(&results), vec!["many.md", "early.md"]);
    }

    /// With the count tied, the note that starts with the term ranks first.
    #[test]
    fn an_earlier_match_outranks_a_later_one() {
        let mut results = vec![row("late.md", &[1000], &[]), row("early.md", &[10], &[])];
        sort_by_relevance(&mut results);
        assert_eq!(names(&results), vec!["early.md", "late.md"]);
    }

    /// A title hit is the strongest place a term can land.
    #[test]
    fn a_name_match_outranks_a_content_only_match() {
        let mut results = vec![row("plain.md", &[10], &[]), row("titled.md", &[], &[0])];
        sort_by_relevance(&mut results);
        assert_eq!(names(&results), vec!["titled.md", "plain.md"]);
    }

    /// Recency is the last tie-break, and only that.
    #[test]
    fn recency_breaks_the_last_tie() {
        let mut results = vec![row("older.md", &[0], &[]), row("newer.md", &[0], &[])];
        results[1].mtime = 500;
        sort_by_relevance(&mut results);
        assert_eq!(names(&results), vec!["newer.md", "older.md"]);
    }

    /// Everything else tied: the name collation, then the path tie, so the
    /// order is deterministic rather than the input's accident.
    #[test]
    fn equal_scores_fall_back_to_name_then_path() {
        let mut results = vec![row("zebra.md", &[40], &[]), row("apple.md", &[40], &[])];
        sort_by_relevance(&mut results);
        assert_eq!(names(&results), vec!["apple.md", "zebra.md"]);
    }

    /// Relevance is a score, and a higher one sorts first: the struct's own
    /// `Ord` is "more relevant is greater", so a caller can compare two
    /// results without sorting a list.
    #[test]
    fn relevance_orders_more_relevant_as_greater() {
        let a = row("a.md", &[10, 500], &[]);
        let b = row("b.md", &[10], &[]);
        assert!(Relevance::of(&a) > Relevance::of(&b));
    }

    /// The other orders remain selectable and unchanged.
    #[test]
    fn the_other_orders_stay_available() {
        let mut results = vec![
            row("zebra.md", &[900], &[]),
            row("apple.md", &[10], &[]),
            row("mango.md", &[500], &[]),
        ];
        results[0].mtime = 10;
        results[1].mtime = 999;
        results[2].mtime = 300;
        results[0].ctime = 10;
        results[1].ctime = 999;
        results[2].ctime = 300;
        sort(&mut results, ResultSort::Alphabetical);
        assert_eq!(names(&results), vec!["apple.md", "mango.md", "zebra.md"]);
        sort(&mut results, ResultSort::AlphabeticalReverse);
        assert_eq!(names(&results), vec!["zebra.md", "mango.md", "apple.md"]);
        sort(&mut results, ResultSort::ByModifiedTime);
        assert_eq!(names(&results), vec!["apple.md", "mango.md", "zebra.md"]);
        sort(&mut results, ResultSort::ByModifiedTimeReverse);
        assert_eq!(names(&results), vec!["zebra.md", "mango.md", "apple.md"]);
        sort(&mut results, ResultSort::ByCreatedTime);
        assert_eq!(names(&results), vec!["apple.md", "mango.md", "zebra.md"]);
        sort(&mut results, ResultSort::ByCreatedTimeReverse);
        assert_eq!(names(&results), vec!["zebra.md", "mango.md", "apple.md"]);
        assert_eq!(ResultSort::default(), ResultSort::Relevance);
    }

    /// The relevance order keeps its result over the list the time orders
    /// produce: the most-matched note first, not the newest.
    #[test]
    fn relevance_differs_from_the_time_orders() {
        let mut results = vec![
            row("new.md", &[100], &[]),
            row("old.md", &[10, 20, 30], &[]),
        ];
        results[0].mtime = 50;
        results[1].mtime = 5;
        sort(&mut results, ResultSort::ByModifiedTime);
        assert_eq!(names(&results), vec!["new.md", "old.md"]);
        sort(&mut results, ResultSort::default());
        assert_eq!(names(&results), vec!["old.md", "new.md"]);
    }
}
