//! Dates and durations, against the 2026-10-25 Obsidian 1.14.5 probe run.
//!
//! Every case here is lifted from
//! `tools/obsidian-probe/results/2026-10-25-bases-expr.json` — steps
//! `e06-values`, `e07-durations`, `e09-globals`, `e14-function-table`,
//! `e16-reduce-and-duration` and `e18-completions`. The clock is pinned to the
//! run's own, so nothing here races a real one.
//!
//! Two things keep the numbers from being a straight copy of the JSON, and
//! both are properties of the crate rather than of the probe:
//!
//! - The crate holds a **civil** time and knows no zone. The run's machine was
//!   at UTC−6 in January and UTC−5 in October, so its `now() - when` is
//!   3600000 ms shorter than the same two civil times are apart. Where that
//!   matters the duration is built from the recorded millisecond count
//!   directly, which is the figure Obsidian's own value object carries.
//! - Each of the eight Duration property reads in `e14` re-evaluated `now()`,
//!   so each landed on its own millisecond. Those are recovered below and
//!   replayed one by one; see the table in `funcs::time`.

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use silica_bases::funcs::time;
use silica_bases::{eval_str, DateValue, DurationValue, FileData, SimpleContext, Value};

/// `now()` as the run's `e14` step read it, in local civil time:
/// `2026-10-25T20:28:50.125Z` at UTC−5.
fn probe_now() -> NaiveDateTime {
    at("2026-10-25T15:28:50.125")
}

fn at(s: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f")
        .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S"))
        .unwrap_or_else(|_| {
            NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .expect("a fixture date")
                .and_time(NaiveTime::MIN)
        })
}

fn date(s: &str) -> DateValue {
    DateValue {
        at: at(s),
        has_time: false,
    }
}

fn datetime(s: &str) -> DateValue {
    DateValue {
        at: at(s),
        has_time: true,
    }
}

fn millis(ms: f64) -> DurationValue {
    DurationValue {
        milliseconds: ms,
        ..DurationValue::default()
    }
}

/// The probe note's frontmatter, as `e06-values` read it back: `when` is the
/// Date `2026-01-02` and `stamp` the DateTime `2026-01-02T03:04:05`.
fn context() -> SimpleContext {
    let mut file = FileData {
        name: "Silica.md".into(),
        basename: "Silica".into(),
        path: "Silica.md".into(),
        ..FileData::default()
    };
    file.properties
        .insert("when".into(), Value::Date(date("2026-01-02")));
    file.properties
        .insert("stamp".into(), Value::Date(datetime("2026-01-02T03:04:05")));
    // `(now() - when)` as the run's `e14` recorded its value object, so a
    // Duration is reachable from a statement without `duration()`.
    file.properties
        .insert("gap".into(), Value::Duration(millis(25_626_530_125.0)));
    SimpleContext::new().with_file(file).with_now(probe_now())
}

/// What a cell shows: the value's own `toString()`, or the refusal Obsidian
/// renders in its place.
fn show(source: &str) -> String {
    match eval_str(source, &context()) {
        Ok(value) => value.to_string(),
        Err(err) => err.message,
    }
}

fn number(value: &Value) -> f64 {
    match value {
        Value::Number(n) => *n,
        other => panic!("expected a Number, got {other:?}"),
    }
}

// --- the Duration's fractional totals ---------------------------------------

/// The headline ratchet. `e14`'s eight reads of one `(now() - when)`, each
/// replayed at the millisecond it was taken on, to the last bit of the double.
#[test]
fn duration_properties_are_the_recorded_fractional_totals() {
    let cases: &[(f64, &str, f64)] = &[
        (25_626_530_121.0, "days", 296.6033578819444),
        (25_626_530_123.0, "hours", 7118.480589722222),
        (25_626_530_125.0, "milliseconds", 25_626_530_125.0),
        (25_626_530_125.0, "minutes", 427_108.83541666664),
        (25_626_530_126.0, "months", 9.761398643219833),
        (25_626_530_128.0, "seconds", 25_626_530.128),
        (25_626_530_131.0, "weeks", 42.3719082853836),
        (25_626_530_131.0, "years", 0.8134498870905515),
    ];
    for (ms, name, want) in cases {
        let value = time::property(&Value::Duration(millis(*ms)), name)
            .unwrap_or_else(|err| panic!("{name}: {err}"));
        assert_eq!(number(&value), *want, "{name} of {ms} ms");
    }
}

/// A month is the month the duration reaches, counting from the epoch, so the
/// round trip through a component Duration is exact: one month is January
/// 1970's 31 days and a year is 1970's 365.
#[test]
fn a_month_is_a_calendar_month_from_the_epoch() {
    let one_month = DurationValue {
        months: 1.0,
        ..DurationValue::default()
    };
    let one_year = DurationValue {
        years: 1.0,
        ..DurationValue::default()
    };
    assert_eq!(time::total_millis(&one_month), 31.0 * 86_400_000.0);
    assert_eq!(time::total_millis(&one_year), 365.0 * 86_400_000.0);
    assert_eq!(
        number(&time::property(&Value::Duration(one_month), "months").unwrap()),
        1.0
    );
    assert_eq!(
        number(&time::property(&Value::Duration(one_year), "years").unwrap()),
        1.0
    );
    // And the SI units stay SI: `duration("1 week")` is `{days: 7}`.
    assert_eq!(
        time::total_millis(&time::parse_duration("1 week").unwrap()),
        604_800_000.0
    );
}

/// Property names are case-insensitive, as every name in the language is, and
/// a Duration has no functions: `(now() - when).days()` is a refusal.
#[test]
fn duration_has_properties_but_no_functions() {
    assert_eq!(show("gap.DAYS"), show("gap.days"));
    assert_eq!(
        show("gap.days()"),
        "Cannot find function \"days\" on type Duration"
    );
    assert_eq!(
        show("gap.years()"),
        "Cannot find function \"years\" on type Duration"
    );
    assert_eq!(
        show("gap.relative()"),
        "Cannot find function \"relative\" on type Duration"
    );
    assert_eq!(
        show("gap.nosuch"),
        "Cannot find \"nosuch\" on type Duration"
    );
    // The dispatcher owns these three, and they reach a Duration.
    assert_eq!(show("gap.toString()"), "10 months");
    assert_eq!(show("gap.isTruthy()"), "true");
}

// --- how a Duration prints --------------------------------------------------

#[test]
fn durations_humanize_as_recorded() {
    let cases: &[(DurationValue, &str)] = &[
        // `duration("1s")`, `("1m")`, `("1h")`, `("1d")`, `("1M")`, `("1y")`.
        (unit(0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0), "a few seconds"),
        (unit(0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0), "a minute"),
        (unit(0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0), "an hour"),
        (unit(0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0), "a day"),
        (unit(0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0), "a month"),
        (unit(1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0), "a year"),
        // `duration("1w")` and `duration("1 week")`, both `{days: 7}`: there
        // is no week rung.
        (unit(0.0, 0.0, 7.0, 0.0, 0.0, 0.0, 0.0), "7 days"),
        // `duration("-1d")`: the sign does not reach the wording.
        (unit(0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0), "a day"),
        // `duration("1d") + duration("1d")` and `duration("1d") * 2`.
        (unit(0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0), "2 days"),
        // `date("2026-01-02") - date("2026-01-01")`.
        (millis(86_400_000.0), "a day"),
        // `now() - when`, as `e07` and `e14` each recorded its value object.
        (millis(25_626_529_922.0), "10 months"),
        (millis(25_626_530_125.0), "10 months"),
    ];
    for (duration, want) in cases {
        assert_eq!(&time::humanize(duration), want, "{duration:?}");
        assert_eq!(&Value::Duration(*duration).to_string(), want);
    }
}

fn unit(
    years: f64,
    months: f64,
    days: f64,
    hours: f64,
    minutes: f64,
    seconds: f64,
    milliseconds: f64,
) -> DurationValue {
    DurationValue {
        years,
        months,
        days,
        hours,
        minutes,
        seconds,
        milliseconds,
    }
}

/// `relative()` — "10 months ago" for the fixture's `stamp`, "a few seconds
/// ago" for the current moment, both recorded in `e14`.
#[test]
fn relative_reads_against_the_clock() {
    assert_eq!(show("stamp.relative()"), "10 months ago");
    let now = DateValue {
        at: probe_now(),
        has_time: true,
    };
    assert_eq!(time::relative(&now, probe_now()), "a few seconds ago");
    // The future half of the same phrasing is unprobed; this pins the choice.
    assert_eq!(
        time::relative(&datetime("2026-10-26T15:28:50.125"), probe_now()),
        "in a day"
    );
}

// --- parsing ----------------------------------------------------------------

#[test]
fn durations_parse_one_number_and_one_unit() {
    let cases: &[(&str, Option<DurationValue>)] = &[
        ("1 week", Some(unit(0.0, 0.0, 7.0, 0.0, 0.0, 0.0, 0.0))),
        ("1w", Some(unit(0.0, 0.0, 7.0, 0.0, 0.0, 0.0, 0.0))),
        ("1y", Some(unit(1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0))),
        ("1M", Some(unit(0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0))),
        ("1m", Some(unit(0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0))),
        ("1d", Some(unit(0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0))),
        ("1h", Some(unit(0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0))),
        ("1s", Some(unit(0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0))),
        ("-1d", Some(unit(0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0))),
        ("1 year", Some(unit(1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0))),
        ("1 month", Some(unit(0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0))),
        ("1 day", Some(unit(0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0))),
        ("1 minute", Some(unit(0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0))),
        // Compound durations are not supported.
        ("1y 2M 3d", None),
        ("not a date", None),
        // Which is why `when - "2026-01-01"` is an operator error.
        ("2026-01-01", None),
        ("", None),
        ("1", None),
        ("week", None),
    ];
    for (source, want) in cases {
        assert_eq!(&time::parse_duration(source), want, "duration({source:?})");
    }
}

#[test]
fn dates_parse_as_written() {
    assert_eq!(time::parse_date("2026-01-02"), Some(date("2026-01-02")));
    assert_eq!(
        time::parse_date("2026-01-02T03:04:05"),
        Some(datetime("2026-01-02T03:04:05"))
    );
    assert_eq!(
        time::parse_date("2026-01-02 03:04"),
        Some(datetime("2026-01-02T03:04:00"))
    );
    // A zone is accepted and dropped: nothing in this crate knows one.
    assert_eq!(
        time::parse_date("2026-01-02T03:04:05Z"),
        Some(datetime("2026-01-02T03:04:05"))
    );
    assert_eq!(time::parse_date("not a date"), None);
    assert_eq!(time::parse_date("1 week"), None);
    assert_eq!(time::parse_date("2026"), None);
    assert_eq!(time::parse_date("2026-02-30"), None);
}

// --- arithmetic -------------------------------------------------------------

/// Every Date expression `e06`, `e07` and `e16` evaluated that does not need a
/// global, run through the parser and the evaluator.
#[test]
fn date_arithmetic_and_comparison() {
    let cases: &[(&str, &str)] = &[
        ("when", "2026-01-02"),
        ("stamp", "2026-01-02T03:04:05"),
        // A String on the Date side of a comparison is parsed as a date.
        ("when > \"2026-01-01\"", "true"),
        ("when == \"2026-01-02\"", "true"),
        ("stamp > when", "true"),
        // But arithmetic wants a Duration, and a date string is not one.
        (
            "when - \"2026-01-01\"",
            "Invalid operator between Date and String",
        ),
        // A String in a Duration's position is parsed as one.
        ("when + \"1 month\"", "2026-02-02"),
        ("when - \"1M\"", "2025-12-02"),
        // `M` is a month where `m` is a minute, and the minute promotes the
        // Date to a DateTime.
        ("when - \"1m\"", "2026-01-01T23:59:00"),
        ("when.year", "2026"),
        ("when.format(\"YYYY\")", "2026"),
        ("stamp.date()", "2026-01-02"),
        ("stamp.time()", "03:04:05"),
        ("stamp.isEmpty()", "false"),
        ("stamp.isTruthy()", "true"),
        ("stamp.format(\"YYYY-MM-DD\")", "2026-01-02"),
        ("stamp.month", "1"),
        ("stamp.day", "2"),
        ("stamp.hour", "3"),
        ("stamp.minute", "4"),
        ("stamp.second", "5"),
        ("stamp.millisecond", "0"),
        // Obsidian type-checks `format`'s parameter before it counts the
        // arguments: `e14` recorded all three of these.
        (
            "stamp.format()",
            "Cannot call function \"format\", not enough arguments.",
        ),
        (
            "stamp.format(/zz/)",
            "Type error in \"format\", parameter \"format\". Expected String, given RegExp.",
        ),
        (
            "stamp.format(/zz/, /zz/)",
            "Type error in \"format\", parameter \"format\". Expected String, given RegExp.",
        ),
        (
            "stamp.nosuch()",
            "Cannot find function \"nosuch\" on type Date",
        ),
    ];
    for (source, want) in cases {
        assert_eq!(&show(source), want, "{source}");
    }
}

/// `date("2026-01-02") - date("2026-01-01")` carries milliseconds only, and
/// `duration + duration`, `duration * 2` and `now() + duration("1d")` all land
/// where `e07` and `e16` recorded them.
#[test]
fn durations_combine_scale_and_shift() {
    let gap = time::between(&date("2026-01-01"), &date("2026-01-02"));
    assert_eq!(gap, millis(86_400_000.0));
    assert_eq!(time::humanize(&gap), "a day");

    let day = time::parse_duration("1d").unwrap();
    assert_eq!(time::humanize(&time::combine(&day, &day, false)), "2 days");
    assert_eq!(time::humanize(&time::scale(&day, 2.0)), "2 days");
    assert_eq!(
        time::humanize(&time::combine(&day, &day, true)),
        "a few seconds"
    );

    // `now() + duration("1d")` → 2026-10-26T15:28:50, still a DateTime.
    let now = DateValue {
        at: probe_now(),
        has_time: true,
    };
    let tomorrow = time::shift(&now, &day, false);
    assert!(tomorrow.has_time);
    assert_eq!(tomorrow.at, at("2026-10-26T15:28:50.125"));
    // `now() - "1 week"`.
    let week = time::parse_duration("1 week").unwrap();
    assert_eq!(
        time::shift(&now, &week, true).at,
        at("2026-10-18T15:28:50.125")
    );
    // `when + duration("1M")` keeps the Date, `when - "1m"` does not.
    let when = date("2026-01-02");
    let month = time::parse_duration("1M").unwrap();
    assert_eq!(
        time::shift(&when, &month, false),
        DateValue {
            at: at("2026-02-02"),
            has_time: false
        }
    );
    assert_eq!(
        time::shift(&when, &time::parse_duration("1m").unwrap(), true),
        DateValue {
            at: at("2026-01-01T23:59:00"),
            has_time: true
        }
    );
    // A month shift clamps to the shorter month rather than overflowing it.
    // Unprobed, and the only answer that stays inside the month.
    assert_eq!(
        time::shift(&date("2026-01-31"), &month, false).at,
        at("2026-02-28")
    );
}

/// The crate's own `now() - when`, which differs from the run's recorded
/// 25626530125 ms by exactly one hour: the probe's interval crossed a daylight
/// saving change and a civil clock has none.
#[test]
fn a_civil_difference_carries_no_daylight_saving() {
    let gap = time::between(
        &date("2026-01-02"),
        &DateValue {
            at: probe_now(),
            has_time: true,
        },
    );
    assert_eq!(gap.milliseconds, 25_626_530_125.0 + 3_600_000.0);
    assert_eq!(time::humanize(&gap), "10 months");
}

/// `now()` has a time and `today()` does not, and `today()` is midnight.
#[test]
fn now_and_today() {
    let now = time::now_of(probe_now(), true);
    let today = time::now_of(probe_now(), false);
    assert_eq!(Value::Date(now).to_string(), "2026-10-25T15:28:50");
    assert_eq!(Value::Date(today).to_string(), "2026-10-25");
    assert_eq!(today.at, at("2026-10-25"));
    assert!(!today.has_time);
}

// --- format() ---------------------------------------------------------------

/// Moment tokens. The run shows only `YYYY-MM-DD` and `YYYY`; the rest of the
/// set is this crate's reading, pinned here so it cannot drift.
#[test]
fn format_renders_moment_tokens() {
    let stamp = datetime("2026-01-02T03:04:05");
    let cases: &[(&str, &str)] = &[
        ("YYYY-MM-DD", "2026-01-02"),
        ("YYYY", "2026"),
        ("YY", "26"),
        ("M/D/YY", "1/2/26"),
        ("MMMM D, YYYY", "January 2, 2026"),
        ("MMM", "Jan"),
        ("Do", "2nd"),
        ("DDD", "2"),
        ("DDDD", "002"),
        ("dddd", "Friday"),
        ("ddd", "Fri"),
        ("dd", "Fr"),
        ("d", "5"),
        ("E", "5"),
        ("Q", "1"),
        ("HH:mm:ss", "03:04:05"),
        ("H:m:s", "3:4:5"),
        ("h:mm A", "3:04 AM"),
        ("hh:mm a", "03:04 am"),
        ("k", "3"),
        ("SSS", "000"),
        ("X", "1767323045"),
        ("x", "1767323045000"),
        ("W", "1"),
        ("WW", "01"),
        ("GGGG", "2026"),
        ("w", "1"),
        ("gggg", "2026"),
        ("Z", "+00:00"),
        ("[on] YYYY", "on 2026"),
        ("L", "01/02/2026"),
        ("LT", "3:04 AM"),
        ("LLLL", "Friday, January 2, 2026 3:04 AM"),
        // Every letter that is a token is one, so plain prose needs escaping.
        ("[no tokens here]", "no tokens here"),
        ("ko", "3o"),
    ];
    for (pattern, want) in cases {
        assert_eq!(&time::format_moment(&stamp, pattern), want, "{pattern}");
    }
    // A Date with no time still formats its midnight.
    assert_eq!(time::format_moment(&date("2026-01-02"), "HH:mm"), "00:00");
}
