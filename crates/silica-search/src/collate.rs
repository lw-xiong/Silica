//! Ordering names the way Obsidian's lists do.
//!
//! Obsidian sorts with a locale-aware, numeric collation (`localeCompare`
//! with `numeric: true`), not by bytes: `Note 2` comes before `Note 10`, and
//! `_needle` before `9 needle` before `apple`. [`natural_cmp`] reproduces the
//! three levels that ordering shows — punctuation before digits before
//! letters, digit runs compared as numbers, case only as a tie-break — over
//! the subset of collation that note names exercise.

use std::cmp::Ordering;

/// Compares two names the way a numeric locale collation does.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut x = a.chars().peekable();
    let mut y = b.chars().peekable();
    // The case difference that decides otherwise equal names, kept until
    // every primary comparison has come out equal.
    let mut tertiary = Ordering::Equal;
    loop {
        let (ca, cb) = match (x.peek().copied(), y.peek().copied()) {
            (Some(ca), Some(cb)) => (ca, cb),
            (None, None) => return tertiary,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
        };
        if ca.is_ascii_digit() && cb.is_ascii_digit() {
            let na = digits(&mut x);
            let nb = digits(&mut y);
            match numeric_cmp(&na, &nb) {
                Ordering::Equal => {
                    // Same number, written differently: `07` after `7`.
                    if tertiary == Ordering::Equal {
                        tertiary = na.len().cmp(&nb.len());
                    }
                }
                other => return other,
            }
            continue;
        }
        match class(ca).cmp(&class(cb)) {
            Ordering::Equal => {}
            other => return other,
        }
        match fold(ca).cmp(&fold(cb)) {
            Ordering::Equal => {}
            other => return other,
        }
        if tertiary == Ordering::Equal && ca != cb {
            // Lowercase sorts before uppercase, the tertiary level of the
            // default collation.
            tertiary = cb.is_lowercase().cmp(&ca.is_lowercase());
        }
        x.next();
        y.next();
    }
}

/// Punctuation and whitespace sort before digits, digits before letters.
fn class(c: char) -> u8 {
    if c.is_alphabetic() {
        2
    } else if c.is_numeric() {
        1
    } else {
        0
    }
}

/// The first char of the lowercase mapping: enough to order names, and it
/// keeps the comparison one char against one char.
fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

fn digits(it: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut out = String::new();
    while let Some(c) = it.peek().copied() {
        if !c.is_ascii_digit() {
            break;
        }
        out.push(c);
        it.next();
    }
    out
}

/// Compares two digit runs as numbers, however long they are.
fn numeric_cmp(a: &str, b: &str) -> Ordering {
    let a = a.trim_start_matches('0');
    let b = b.trim_start_matches('0');
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digit_runs_compare_as_numbers() {
        assert_eq!(natural_cmp("Note 2", "Note 10"), Ordering::Less);
        assert_eq!(natural_cmp("apple10", "apple2"), Ordering::Greater);
        assert_eq!(natural_cmp("a9b", "a10b"), Ordering::Less);
        assert_eq!(natural_cmp("v1.9", "v1.10"), Ordering::Less);
        assert_eq!(natural_cmp("7", "007"), Ordering::Less);
        assert_eq!(natural_cmp("12", "12"), Ordering::Equal);
        assert_eq!(
            natural_cmp("9999999999999999999999", "10000000000000000000000"),
            Ordering::Less
        );
    }

    #[test]
    fn punctuation_then_digits_then_letters_and_case_last() {
        assert_eq!(natural_cmp("_needle", "9 needle"), Ordering::Less);
        assert_eq!(natural_cmp("9 needle", "apple"), Ordering::Less);
        assert_eq!(natural_cmp("apple", "Apple Pie"), Ordering::Less);
        assert_eq!(natural_cmp("Apple Pie", "apple2"), Ordering::Less);
        assert_eq!(natural_cmp("apple", "APPLE"), Ordering::Less);
        assert_eq!(natural_cmp("apple", "apple"), Ordering::Equal);
        assert_eq!(natural_cmp("Ápple", "Apple"), Ordering::Greater);
    }

    /// The order a real Obsidian 1.14.4 produced for these names under
    /// `alphabetical`, captured in tools/obsidian-probe/results and written
    /// up in docs/silica/Specification.md.
    #[test]
    fn reproduces_the_observed_obsidian_order() {
        let mut names = vec![
            "zebra",
            "apple2",
            "Apple Pie",
            "kiwi",
            "9 needle",
            "apple",
            "mango",
            "_needle",
            "banana",
            "apple10",
        ];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            names,
            vec![
                "_needle",
                "9 needle",
                "apple",
                "Apple Pie",
                "apple2",
                "apple10",
                "banana",
                "kiwi",
                "mango",
                "zebra",
            ]
        );
    }
}
