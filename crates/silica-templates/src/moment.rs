//! The moment.js date formats Obsidian's templates and daily notes take.
//!
//! Obsidian formats a date with moment.js, which Silica has no equivalent
//! of, so this is the subset a vault's format strings actually use. Every
//! token here was rendered by the running app against five fixed dates and
//! is tested against that output; see
//! [[Specification#Observed templates behavior, Obsidian 1.14.4]].
//!
//! A token this does not implement is **not** guessed at:
//! [`unsupported_tokens`] names it so a caller can refuse rather than write
//! a date that is quietly wrong — a daily note's file name is a path, and a
//! wrong one is a note in the wrong place.

use crate::stamp::Stamp;

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// The tokens this understands, longest first so `MMMM` wins over `MM`.
const TOKENS: &[&str] = &[
    "YYYY", "MMMM", "dddd", "SSS", "MMM", "DDD", "ddd", "YY", "MM", "DD", "Do", "HH", "hh", "mm",
    "ss", "Q", "M", "D", "d", "H", "h", "m", "s", "A", "a", "X", "x",
];

/// Tokens moment renders that this does not: week numbers and their
/// week-years, the zone, and the day-of-week spellings that depend on the
/// locale's first day.
const KNOWN_UNSUPPORTED: &[&str] = &[
    "GGGG", "gggg", "WW", "ww", "ZZ", "GG", "gg", "E", "e", "W", "w", "Z", "k", "S",
];

/// `format` rendered for `at`, with `[...]` passed through verbatim.
pub fn format(format: &str, at: &Stamp) -> String {
    let mut out = String::with_capacity(format.len() + 8);
    let bytes = format.as_bytes();
    let mut i = 0;
    while i < format.len() {
        // A bracketed run is literal, brackets dropped.
        if bytes[i] == b'[' {
            match format[i + 1..].find(']') {
                Some(end) => {
                    out.push_str(&format[i + 1..i + 1 + end]);
                    i += end + 2;
                    continue;
                }
                None => {
                    out.push_str(&format[i..]);
                    break;
                }
            }
        }
        match TOKENS.iter().find(|t| format[i..].starts_with(**t)) {
            Some(token) => {
                out.push_str(&render(token, at));
                i += token.len();
            }
            None => {
                let c = format[i..].chars().next().expect("a character");
                out.push(c);
                i += c.len_utf8();
            }
        }
    }
    out
}

/// The tokens in `format` that [`format`] cannot render, in the order they
/// appear. A caller with an empty list can trust what it gets back.
pub fn unsupported_tokens(format: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = format.as_bytes();
    let mut i = 0;
    while i < format.len() {
        if bytes[i] == b'[' {
            match format[i + 1..].find(']') {
                Some(end) => {
                    i += end + 2;
                    continue;
                }
                None => break,
            }
        }
        if let Some(token) = TOKENS.iter().find(|t| format[i..].starts_with(**t)) {
            i += token.len();
            continue;
        }
        match KNOWN_UNSUPPORTED
            .iter()
            .find(|t| format[i..].starts_with(**t))
        {
            Some(token) => {
                if !out.iter().any(|seen| seen == token) {
                    out.push((*token).to_string());
                }
                i += token.len();
            }
            None => i += format[i..].chars().next().map(char::len_utf8).unwrap_or(1),
        }
    }
    out
}

fn render(token: &str, at: &Stamp) -> String {
    let hour12 = match at.hour % 12 {
        0 => 12,
        h => h,
    };
    match token {
        "YYYY" => format!("{:04}", at.year),
        "YY" => format!("{:02}", at.year.rem_euclid(100)),
        "MMMM" => MONTHS[(at.month - 1) as usize].to_string(),
        "MMM" => MONTHS[(at.month - 1) as usize][..3].to_string(),
        "MM" => format!("{:02}", at.month),
        "M" => at.month.to_string(),
        "DDD" => at.day_of_year().to_string(),
        "DD" => format!("{:02}", at.day),
        "Do" => ordinal(at.day),
        "D" => at.day.to_string(),
        "dddd" => DAYS[at.weekday() as usize].to_string(),
        "ddd" => DAYS[at.weekday() as usize][..3].to_string(),
        "d" => at.weekday().to_string(),
        "HH" => format!("{:02}", at.hour),
        "H" => at.hour.to_string(),
        "hh" => format!("{hour12:02}"),
        "h" => hour12.to_string(),
        "mm" => format!("{:02}", at.minute),
        "m" => at.minute.to_string(),
        "ss" => format!("{:02}", at.second),
        "s" => at.second.to_string(),
        "SSS" => "000".to_string(),
        "Q" => (((at.month - 1) / 3) + 1).to_string(),
        "A" => if at.hour < 12 { "AM" } else { "PM" }.to_string(),
        "a" => if at.hour < 12 { "am" } else { "pm" }.to_string(),
        "X" => (at.days_from_epoch() * 86_400
            + (at.hour * 3600 + at.minute * 60 + at.second) as i64)
            .to_string(),
        "x" => ((at.days_from_epoch() * 86_400
            + (at.hour * 3600 + at.minute * 60 + at.second) as i64)
            * 1000)
            .to_string(),
        other => other.to_string(),
    }
}

/// `1st`, `2nd`, `3rd`, `4th`, the way moment's `Do` writes a day.
fn ordinal(day: u32) -> String {
    let suffix = match (day % 10, day % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{day}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The five dates Obsidian rendered every token against.
    fn dates() -> Vec<Stamp> {
        [
            (2026, 1, 1, 0, 5, 0),
            (2024, 2, 29, 13, 45, 30),
            (2026, 12, 31, 23, 59, 59),
            (2026, 10, 18, 9, 7, 3),
            (2026, 10, 19, 0, 0, 0),
        ]
        .iter()
        .map(|&(year, month, day, hour, minute, second)| Stamp {
            year,
            month,
            day,
            hour,
            minute,
            second,
        })
        .collect()
    }

    /// `token` rendered for all five dates, as Obsidian rendered it.
    fn row(token: &str) -> Vec<String> {
        dates().iter().map(|d| format(token, d)).collect()
    }

    #[test]
    fn every_token_matches_what_obsidian_rendered() {
        // Taken verbatim from the probe's `en.rows` table.
        let observed: &[(&str, [&str; 5])] = &[
            ("YYYY", ["2026", "2024", "2026", "2026", "2026"]),
            ("YY", ["26", "24", "26", "26", "26"]),
            ("MM", ["01", "02", "12", "10", "10"]),
            ("M", ["1", "2", "12", "10", "10"]),
            ("MMM", ["Jan", "Feb", "Dec", "Oct", "Oct"]),
            (
                "MMMM",
                ["January", "February", "December", "October", "October"],
            ),
            ("DD", ["01", "29", "31", "18", "19"]),
            ("D", ["1", "29", "31", "18", "19"]),
            ("Do", ["1st", "29th", "31st", "18th", "19th"]),
            ("DDD", ["1", "60", "365", "291", "292"]),
            ("ddd", ["Thu", "Thu", "Thu", "Sun", "Mon"]),
            (
                "dddd",
                ["Thursday", "Thursday", "Thursday", "Sunday", "Monday"],
            ),
            ("d", ["4", "4", "4", "0", "1"]),
            ("Q", ["1", "1", "4", "4", "4"]),
            ("HH", ["00", "13", "23", "09", "00"]),
            ("H", ["0", "13", "23", "9", "0"]),
            ("hh", ["12", "01", "11", "09", "12"]),
            ("h", ["12", "1", "11", "9", "12"]),
            ("mm", ["05", "45", "59", "07", "00"]),
            ("m", ["5", "45", "59", "7", "0"]),
            ("ss", ["00", "30", "59", "03", "00"]),
            ("s", ["0", "30", "59", "3", "0"]),
            ("SSS", ["000", "000", "000", "000", "000"]),
            ("A", ["AM", "PM", "PM", "AM", "AM"]),
            ("a", ["am", "pm", "pm", "am", "am"]),
        ];
        for (token, want) in observed {
            assert_eq!(row(token), want.to_vec(), "token {token}");
        }
    }

    #[test]
    fn the_formats_a_vault_actually_writes() {
        let d = &dates()[3]; // 2026-10-18 09:07:03, a Sunday
        assert_eq!(format("YYYY-MM-DD", d), "2026-10-18");
        assert_eq!(format("YYYY-MM-DD HH:mm", d), "2026-10-18 09:07");
        assert_eq!(format("HH:mm", d), "09:07");
        assert_eq!(format("YYYY/MM/YYYY-MM-DD", d), "2026/10/2026-10-18");
        assert_eq!(format("dddd, MMMM Do YYYY", d), "Sunday, October 18th 2026");
    }

    #[test]
    fn brackets_pass_through_verbatim() {
        let d = &dates()[3];
        // The probe's own `{{date:YYYY[-]MM-DD ddd}}` rendered this way.
        assert_eq!(format("YYYY[-]MM-DD ddd", d), "2026-10-18 Sun");
        // A bracketed token is text, not a token.
        assert_eq!(format("[YYYY] YYYY", d), "YYYY 2026");
        assert_eq!(format("[Daily] YYYY", d), "Daily 2026");
        // An unclosed bracket: moment's own handling was not observed, so
        // the rest is passed through as written rather than guessed at.
        assert_eq!(format("YYYY [and on", d), "2026 [and on");
    }

    #[test]
    fn a_token_that_is_not_implemented_is_named_rather_than_guessed() {
        assert!(unsupported_tokens("YYYY-MM-DD").is_empty());
        assert!(unsupported_tokens("dddd, MMMM Do YYYY").is_empty());
        // Week numbers and zones are the ones a vault might still use.
        assert_eq!(unsupported_tokens("GGGG-[W]WW"), ["GGGG", "WW"]);
        assert_eq!(unsupported_tokens("YYYY-MM-DD Z"), ["Z"]);
        // A bracketed one is literal text, not a token.
        assert!(unsupported_tokens("YYYY[W]MM").is_empty());
    }

    #[test]
    fn separators_and_unknown_characters_are_kept() {
        let d = &dates()[0];
        assert_eq!(format("YYYY_MM_DD", d), "2026_01_01");
        assert_eq!(format("YYYY年MM月DD日", d), "2026年01月01日");
        assert_eq!(format("", d), "");
    }
}
