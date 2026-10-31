//! Date, DateTime and Duration — and every conversion the rest of the crate
//! needs, which is why the free functions below are the module's real surface.
//!
//! | Receiver | Functions | Properties |
//! | --- | --- | --- |
//! | Date, DateTime | `date()` (drops the time), `format(format: String)` (Moment.js), `isEmpty()`, `relative()`, `time()` | `year`, `month`, `day`, `hour`, `minute`, `second`, `millisecond`, `timestamp` |
//! | Duration | — | `years`, `months`, `weeks`, `days`, `hours`, `minutes`, `seconds`, `milliseconds` |
//!
//! Observed and easy to get wrong:
//!
//! - A Duration's properties are **fractional totals**, not components.
//! - The unit letters are `y M w d h m s`, and **`M` is a month where `m` is a
//!   minute**: `when - "1M"` is 2025-12-02 where `when - "1m"` is
//!   2026-01-01 23:59 — and the minute *promotes a Date to a DateTime*.
//! - `duration("1w")` and `duration("1 week")` agree, and both answer
//!   `{days: 7}`: a week is not a component. `duration("1y 2M 3d")` is
//!   **null**, so compound durations are not supported.
//! - A Duration prints humanized: "10 months", "7 days", "a day", "a minute",
//!   "a few seconds".
//! - Dates here are civil: Obsidian parses `"2026-01-02"` to local midnight.
//!
//! ## The month, and where the numbers come from
//!
//! Every Duration property but `months` and `years` is an exact SI division of
//! the total milliseconds. Those two are **calendar** quantities, and the
//! 2026-10-25 run pins the rule down bit for bit. Its eight property reads of
//! one `(now() - when)` each re-evaluated `now()`, so each lands on its own
//! millisecond; recovering that millisecond from the exact conversions and
//! replaying it gives, in the run's own reading order:
//!
//! | Read | Milliseconds | Recorded value |
//! | --- | --- | --- |
//! | `days` | 25626530121 | `296.6033578819444` = ms / 86400000 |
//! | `hours` | 25626530123 | `7118.480589722222` = ms / 3600000 |
//! | `milliseconds` | 25626530125 | `25626530125` |
//! | `minutes` | 25626530125 | `427108.83541666664` = ms / 60000 |
//! | `months` | 25626530126 | `9.761398643219833` |
//! | `seconds` | 25626530128 | `25626530.128` = ms / 1000 |
//! | `weeks` | 25626530131 | `42.3719082853836` = ms / 604800000 |
//! | `years` | 25626530131 | `0.8134498870905515` = months / 12 |
//!
//! The reads are monotonic and 0–3 ms apart, which is what a sequence of
//! re-evaluated clock reads looks like, and `months` falls out exactly when the
//! duration is laid on the calendar **from the Unix epoch**: 25626530126 ms
//! after 1970-01-01T00:00:00 is 1970-10-24T14:28:50.126, which is 9 whole
//! months (January through September, 273 days) plus 23.603… days of an
//! October 31 days long, and `9 + 2039330126/2678400000` is
//! `9.761398643219833` to the last bit. `years` is then exactly `months / 12`,
//! which is why a year is worth 365 days here (1970 is not a leap year) and a
//! month 31 (January is). No fixed day count reproduces those figures: the
//! usual candidates give 9.7449 (30.436875 days), 9.7513 (365/12) or 9.8868
//! (30 days) where Obsidian gives 9.7614.
//!
//! So: **a month is however long the month is that the duration reaches,
//! counting from the epoch**, and [`months_to_millis`] is the exact inverse,
//! which is what gives a Duration built out of components — `duration("1M")` —
//! a total to be compared and humanized by.
//!
//! Everything marked `unprobed` below is a reading the run did not settle.

use chrono::{Datelike, Months, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, Timelike};

use crate::eval::{EvalResult, Evaluator};
use crate::expr::Expr;
use crate::value::{js_number, DateValue, DurationValue, Value};

use super::{arity, not_enough, too_many, unknown_function, unknown_property, want_string};

const MS_SECOND: f64 = 1_000.0;
const MS_MINUTE: f64 = 60_000.0;
const MS_HOUR: f64 = 3_600_000.0;
const MS_DAY: f64 = 86_400_000.0;
const MS_WEEK: f64 = 604_800_000.0;

/// The Gregorian mean month, used only where the calendar walk cannot reach —
/// a month count no `NaiveDateTime` can hold. Nothing a vault contains gets
/// there.
const MS_MONTH_FALLBACK: f64 = 2_629_746_000.0;

// --- the dispatcher's two entry points --------------------------------------

/// A method call on a Date, a DateTime or a Duration. Duration has no
/// functions of its own: `(now() - when).days()` is
/// `Cannot find function "days" on type Duration`, and so is `relative()`.
pub fn call(ev: &mut Evaluator<'_>, subject: &Value, name: &str, args: &[Expr]) -> EvalResult {
    let Value::Date(date) = subject else {
        return Err(unknown_function(name, subject.ty()));
    };
    match name.to_ascii_lowercase().as_str() {
        // "Returns a date object with the time removed."
        "date" => {
            arity(name, args, 0, 0)?;
            Ok(Value::Date(DateValue {
                at: date.at.date().and_time(NaiveTime::MIN),
                has_time: false,
            }))
        }
        // Obsidian type-checks the format *before* it counts the arguments:
        // `format(/zz/, /zz/)` answers the type error, not "too many
        // arguments", where `format()` answers "not enough arguments". The
        // order below is the observed one; that a second argument is refused
        // at all once the first is a String is `unprobed`.
        "format" => {
            if args.is_empty() {
                return Err(not_enough(name));
            }
            let arg = ev.eval(&args[0])?;
            let pattern = want_string(name, "format", &arg)?;
            if args.len() > 1 {
                return Err(too_many(name));
            }
            Ok(Value::Str(format_moment(date, &pattern)))
        }
        // "Returns false." A Date is never empty.
        "isempty" => {
            arity(name, args, 0, 0)?;
            Ok(Value::Bool(false))
        }
        // "Returns a readable comparison of the date to the current datetime."
        "relative" => {
            arity(name, args, 0, 0)?;
            Ok(Value::Relative(relative(date, ev.context().now())))
        }
        // "Returns the time." Observed as `15:28:49` — `HH:mm:ss`, a String.
        "time" => {
            arity(name, args, 0, 0)?;
            Ok(Value::Str(format!(
                "{:02}:{:02}:{:02}",
                date.at.hour(),
                date.at.minute(),
                date.at.second()
            )))
        }
        _ => Err(unknown_function(name, subject.ty())),
    }
}

/// A property of a Date, a DateTime or a Duration. Names are case-insensitive,
/// as every name in the language is.
pub fn property(subject: &Value, name: &str) -> EvalResult {
    let found = match subject {
        Value::Date(date) => date_property(date, name),
        Value::Duration(dur) => duration_property(dur, name),
        _ => None,
    };
    found
        .map(Value::Number)
        .ok_or_else(|| unknown_property(name, subject.ty()))
}

/// `year`, `month` (**1-based**: January is 1), `day`, `hour`, `minute`,
/// `second`, `millisecond`, `timestamp`.
fn date_property(date: &DateValue, name: &str) -> Option<f64> {
    let at = date.at;
    Some(match name.to_ascii_lowercase().as_str() {
        "year" => f64::from(at.year()),
        "month" => f64::from(at.month()),
        "day" => f64::from(at.day()),
        "hour" => f64::from(at.hour()),
        "minute" => f64::from(at.minute()),
        "second" => f64::from(at.second()),
        "millisecond" => f64::from(at.nanosecond() / 1_000_000),
        // `unprobed`: the run lists `timestamp` in the completion set but
        // never read one. Epoch milliseconds is what the neighbouring
        // `millisecond` makes of it, and the civil time is read as UTC —
        // the crate knows no zone, so a real Obsidian would differ by the
        // viewer's offset.
        "timestamp" => at.and_utc().timestamp_millis() as f64,
        _ => return None,
    })
}

/// `years`, `months`, `weeks`, `days`, `hours`, `minutes`, `seconds`,
/// `milliseconds` — every one a fractional **total**, not a component.
fn duration_property(dur: &DurationValue, name: &str) -> Option<f64> {
    let ms = total_millis(dur);
    Some(match name.to_ascii_lowercase().as_str() {
        "years" => months_of_millis(ms) / 12.0,
        "months" => months_of_millis(ms),
        "weeks" => ms / MS_WEEK,
        "days" => ms / MS_DAY,
        "hours" => ms / MS_HOUR,
        "minutes" => ms / MS_MINUTE,
        "seconds" => ms / MS_SECOND,
        "milliseconds" => ms,
        _ => return None,
    })
}

// --- parsing ----------------------------------------------------------------

/// `date("…")`, and the String side of a Date comparison (`when >
/// "2026-01-01"`, `when == "2026-01-02"`). `date("not a date")` is null.
///
/// `"2026-01-02"` is a Date and `"2026-01-02T03:04:05"` a DateTime: the `time`
/// flag follows the *written form*, not the clock reading, so a date-time at
/// midnight stays a DateTime.
///
/// `unprobed`: which shapes beyond these two Obsidian accepts. A date part is
/// required here, so `date("2026")` is null; a trailing `Z` or `±HH:MM` is
/// accepted and **ignored**, because nothing in this crate knows a zone.
pub fn parse_date(s: &str) -> Option<DateValue> {
    let text = s.trim();
    if text.len() < 10 || !text.is_char_boundary(10) {
        return None;
    }
    let (head, mut rest) = text.split_at(10);
    let date = NaiveDate::parse_from_str(head, "%Y-%m-%d").ok()?;
    if rest.is_empty() {
        return Some(DateValue {
            at: date.and_time(NaiveTime::MIN),
            has_time: false,
        });
    }

    // A separator, then at least `HH:mm`.
    let sep = rest.as_bytes()[0];
    if sep != b'T' && sep != b't' && sep != b' ' {
        return None;
    }
    rest = &rest[1..];
    let (time, tail) = split_zone(rest);
    let parsed = NaiveTime::parse_from_str(time, "%H:%M:%S%.f")
        .or_else(|_| NaiveTime::parse_from_str(time, "%H:%M"))
        .ok()?;
    if !tail.is_empty() && !is_zone(tail) {
        return None;
    }
    Some(DateValue {
        at: date.and_time(parsed),
        has_time: true,
    })
}

/// Splits a time off the zone suffix that may follow it.
fn split_zone(rest: &str) -> (&str, &str) {
    for (i, c) in rest.char_indices() {
        if c == 'Z' || c == 'z' || c == '+' || (c == '-' && i > 0) {
            return (&rest[..i], &rest[i..]);
        }
    }
    (rest, "")
}

fn is_zone(tail: &str) -> bool {
    if tail == "Z" || tail == "z" {
        return true;
    }
    let bytes = tail.as_bytes();
    if bytes[0] != b'+' && bytes[0] != b'-' {
        return false;
    }
    let digits: String = tail[1..].chars().filter(|c| *c != ':').collect();
    matches!(digits.len(), 2 | 4) && digits.chars().all(|c| c.is_ascii_digit())
}

/// `duration("…")`, and the String side of Date arithmetic (`now() -
/// "1 week"`). A duration is **one** number and **one** unit: `"1y 2M 3d"` is
/// null, and so is `"2026-01-01"`, which is why `when - "2026-01-01"` is
/// `Invalid operator between Date and String` rather than a date shift.
///
/// The one-letter units are case-sensitive — `M` is a month, `m` a minute —
/// and a week is folded into days at once, because `duration("1 week")`
/// answers `{days: 7}`.
///
/// `unprobed`: that the spelled-out units are case-insensitive, that a
/// fractional count (`"1.5d"`) is accepted, and `"ms"` for milliseconds, which
/// is here because the component exists.
pub fn parse_duration(s: &str) -> Option<DurationValue> {
    let text = s.trim();
    let digits = text
        .char_indices()
        .take_while(|(i, c)| {
            c.is_ascii_digit() || *c == '.' || (*i == 0 && (*c == '-' || *c == '+'))
        })
        .count();
    if digits == 0 {
        return None;
    }
    let (number, unit) = text.split_at(digits);
    let count: f64 = number.parse().ok()?;
    if !count.is_finite() {
        return None;
    }
    let unit = unit.trim();
    let mut out = DurationValue::default();
    let field = if unit.chars().count() == 1 {
        // Case-sensitive, and the whole point of the family.
        match unit {
            "y" => Unit::Years,
            "M" => Unit::Months,
            "w" => Unit::Weeks,
            "d" => Unit::Days,
            "h" => Unit::Hours,
            "m" => Unit::Minutes,
            "s" => Unit::Seconds,
            _ => return None,
        }
    } else {
        match unit.to_ascii_lowercase().as_str() {
            "year" | "years" => Unit::Years,
            "month" | "months" => Unit::Months,
            "week" | "weeks" => Unit::Weeks,
            "day" | "days" => Unit::Days,
            "hour" | "hours" => Unit::Hours,
            "minute" | "minutes" => Unit::Minutes,
            "second" | "seconds" => Unit::Seconds,
            "ms" | "millisecond" | "milliseconds" => Unit::Milliseconds,
            _ => return None,
        }
    };
    match field {
        Unit::Years => out.years = count,
        Unit::Months => out.months = count,
        Unit::Weeks => out.days = count * 7.0,
        Unit::Days => out.days = count,
        Unit::Hours => out.hours = count,
        Unit::Minutes => out.minutes = count,
        Unit::Seconds => out.seconds = count,
        Unit::Milliseconds => out.milliseconds = count,
    }
    Some(out)
}

enum Unit {
    Years,
    Months,
    Weeks,
    Days,
    Hours,
    Minutes,
    Seconds,
    Milliseconds,
}

// --- printing ---------------------------------------------------------------

/// How a Duration prints, and what `toString()` answers.
///
/// The ladder below is read off the run, which shows every rung it reaches:
/// 1 s "a few seconds", 1 min "a minute", 1 h "an hour", 1 d and −1 d and
/// 86400000 ms "a day", 2 d "2 days", 7 d "7 days" (**not** "a week": there is
/// no week rung), 1 M "a month", 296.6 d "10 months", 1 y "a year". The
/// magnitude decides, and the sign does not — a negative duration humanizes by
/// its absolute value.
///
/// `unprobed`: every *threshold* between those rungs, and the plural wordings
/// "N minutes", "N hours" and "N years", which no example reached. The
/// boundaries used here — 44 s, 44 min, 21 h, 25 d, 10 months — are the ones
/// consistent with all nine observed rungs, and in particular with 296.6 days
/// rounding to "10 months" rather than printing as days.
pub fn humanize(d: &DurationValue) -> String {
    humanize_millis(total_millis(d).abs())
}

fn humanize_millis(ms: f64) -> String {
    if !ms.is_finite() {
        return "a few seconds".into();
    }
    let seconds = (ms / MS_SECOND).round();
    if seconds <= 44.0 {
        return "a few seconds".into();
    }
    let minutes = (ms / MS_MINUTE).round();
    if minutes <= 1.0 {
        return "a minute".into();
    }
    if minutes <= 44.0 {
        return format!("{} minutes", js_number(minutes));
    }
    let hours = (ms / MS_HOUR).round();
    if hours <= 1.0 {
        return "an hour".into();
    }
    if hours <= 21.0 {
        return format!("{} hours", js_number(hours));
    }
    let days = (ms / MS_DAY).round();
    if days <= 1.0 {
        return "a day".into();
    }
    if days <= 25.0 {
        return format!("{} days", js_number(days));
    }
    let months = months_of_millis(ms).round();
    if months <= 1.0 {
        return "a month".into();
    }
    if months <= 10.0 {
        return format!("{} months", js_number(months));
    }
    let years = (months_of_millis(ms) / 12.0).round();
    if years <= 1.0 {
        return "a year".into();
    }
    format!("{} years", js_number(years))
}

/// `relative()` — "a readable comparison of the date to the current
/// datetime". Observed as "10 months ago" for a date ten months back and
/// "a few seconds ago" for `now()` itself, so a zero difference reads as past.
///
/// `unprobed`: the future wording. "in X" is the other half of the same
/// phrasing and is what is used here.
pub fn relative(date: &DateValue, now: NaiveDateTime) -> String {
    let delta = millis_between(now, date.at);
    let text = humanize_millis(delta.abs());
    if delta > 0.0 {
        format!("in {text}")
    } else {
        format!("{text} ago")
    }
}

// --- arithmetic -------------------------------------------------------------

/// A Duration as a single number, for comparison and for the fractional total
/// properties. Years and months go through the calendar from the epoch, which
/// is what makes `duration("1M")` 31 days and `duration("1y")` 365; everything
/// else is a fixed multiple.
pub fn total_millis(d: &DurationValue) -> f64 {
    months_to_millis(d.years * 12.0 + d.months)
        + d.days * MS_DAY
        + d.hours * MS_HOUR
        + d.minutes * MS_MINUTE
        + d.seconds * MS_SECOND
        + d.milliseconds
}

/// `date + duration` and `date - duration`.
///
/// Years and months move the *calendar*: `when + "1 month"` is 2026-02-02 and
/// `when - "1M"` is 2025-12-02, never 30 or 31 days. Days and smaller are a
/// fixed offset.
///
/// A sub-day unit **promotes a Date to a DateTime**: `when - "1m"` is
/// `2026-01-01T23:59:00` with `time: true`, where `when - "1M"` and
/// `when + duration("1M")` stay Dates. The flag therefore follows the units
/// used, not whether the result lands on midnight — `unprobed`, but the
/// reading that cannot silently drop a DateTime's own flag.
pub fn shift(d: &DateValue, by: &DurationValue, subtract: bool) -> DateValue {
    let sign = if subtract { -1.0 } else { 1.0 };
    let months = sign * (by.years * 12.0 + by.months);
    let rest = sign
        * (by.days * MS_DAY
            + by.hours * MS_HOUR
            + by.minutes * MS_MINUTE
            + by.seconds * MS_SECOND
            + by.milliseconds);
    let mut at = add_months_fractional(d.at, months);
    at = add_millis(at, rest);
    DateValue {
        at,
        has_time: d.has_time || touches_time(by),
    }
}

/// Whether a duration reaches below a whole day, which is what promotes a Date
/// to a DateTime.
fn touches_time(by: &DurationValue) -> bool {
    by.hours != 0.0
        || by.minutes != 0.0
        || by.seconds != 0.0
        || by.milliseconds != 0.0
        || by.days.fract() != 0.0
        || by.months.fract() != 0.0
        || by.years.fract() != 0.0
}

/// `later - earlier`. The result carries **milliseconds only**, which is what
/// Obsidian's own object does: `date("2026-01-02") - date("2026-01-01")` is
/// `{milliseconds: 86400000}` with every other component 0.
pub fn between(from: &DateValue, to: &DateValue) -> DurationValue {
    DurationValue {
        milliseconds: millis_between(from.at, to.at),
        ..DurationValue::default()
    }
}

/// `duration + duration` and `duration - duration`, component by component:
/// `duration("1d") + duration("1d")` is "2 days".
pub fn combine(a: &DurationValue, b: &DurationValue, subtract: bool) -> DurationValue {
    let sign = if subtract { -1.0 } else { 1.0 };
    DurationValue {
        years: a.years + sign * b.years,
        months: a.months + sign * b.months,
        days: a.days + sign * b.days,
        hours: a.hours + sign * b.hours,
        minutes: a.minutes + sign * b.minutes,
        seconds: a.seconds + sign * b.seconds,
        milliseconds: a.milliseconds + sign * b.milliseconds,
    }
}

/// `duration * n` and `duration / n`: `duration("1d") * 2` is "2 days".
pub fn scale(d: &DurationValue, factor: f64) -> DurationValue {
    DurationValue {
        years: d.years * factor,
        months: d.months * factor,
        days: d.days * factor,
        hours: d.hours * factor,
        minutes: d.minutes * factor,
        seconds: d.seconds * factor,
        milliseconds: d.milliseconds * factor,
    }
}

/// A civil date-time from the host's clock. `now()` has a time and `today()`
/// does not; `today()` is midnight.
pub fn now_of(ctx_now: NaiveDateTime, has_time: bool) -> DateValue {
    let at = if has_time {
        ctx_now
    } else {
        ctx_now.date().and_time(NaiveTime::MIN)
    };
    DateValue { at, has_time }
}

// --- the calendar -----------------------------------------------------------

fn epoch() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(1970, 1, 1)
        .expect("1970-01-01 is a date")
        .and_time(NaiveTime::MIN)
}

fn millis_between(from: NaiveDateTime, to: NaiveDateTime) -> f64 {
    (to - from).num_milliseconds() as f64
}

fn add_millis(at: NaiveDateTime, ms: f64) -> NaiveDateTime {
    if !ms.is_finite() {
        return at;
    }
    TimeDelta::try_milliseconds(ms.round() as i64)
        .and_then(|delta| at.checked_add_signed(delta))
        .unwrap_or(at)
}

fn add_months_i(at: NaiveDateTime, n: i64) -> Option<NaiveDateTime> {
    let step = u32::try_from(n.unsigned_abs()).ok()?;
    if n >= 0 {
        at.checked_add_months(Months::new(step))
    } else {
        at.checked_sub_months(Months::new(step))
    }
}

/// A whole number of calendar months, plus a fraction of the month the result
/// lands in. Obsidian's own durations only ever carry whole months, so the
/// fraction is `unprobed`; this is the continuous extension of the whole case.
fn add_months_fractional(at: NaiveDateTime, months: f64) -> NaiveDateTime {
    if months == 0.0 || !months.is_finite() {
        return at;
    }
    let whole = months.trunc();
    let frac = months - whole;
    let base = add_months_i(at, whole as i64).unwrap_or(at);
    if frac == 0.0 {
        return base;
    }
    let step = if frac > 0.0 { 1 } else { -1 };
    let neighbour = add_months_i(base, step).unwrap_or(base);
    let span = millis_between(base, neighbour).abs();
    add_millis(base, frac * span)
}

/// The first instant of the month `at` falls in.
fn month_start(at: NaiveDateTime) -> NaiveDateTime {
    at.date()
        .with_day(1)
        .unwrap_or(at.date())
        .and_time(NaiveTime::MIN)
}

/// Milliseconds as fractional calendar months, laid out from the epoch. See
/// the module header: this is the rule the run's `months` and `years` pin down
/// exactly.
fn months_of_millis(ms: f64) -> f64 {
    if !ms.is_finite() {
        return ms / MS_MONTH_FALLBACK;
    }
    let whole = ms.floor();
    let frac = ms - whole;
    let Some(at) = TimeDelta::try_milliseconds(whole as i64)
        .and_then(|delta| epoch().checked_add_signed(delta))
    else {
        return ms / MS_MONTH_FALLBACK;
    };
    let start = month_start(at);
    let Some(next) = add_months_i(start, 1) else {
        return ms / MS_MONTH_FALLBACK;
    };
    let span = millis_between(start, next);
    let n = i64::from(at.year() - 1970) * 12 + i64::from(at.month()) - 1;
    n as f64 + (millis_between(start, at) + frac) / span
}

/// The exact inverse of [`months_of_millis`]: what a Duration's own `months`
/// and `years` components are worth. One month is January 1970 — 31 days — and
/// twelve are 1970 — 365.
pub fn months_to_millis(months: f64) -> f64 {
    if months == 0.0 {
        return 0.0;
    }
    if !months.is_finite() {
        return months * MS_MONTH_FALLBACK;
    }
    let whole = months.floor();
    let frac = months - whole;
    let base = epoch();
    let Some(start) = add_months_i(base, whole as i64) else {
        return months * MS_MONTH_FALLBACK;
    };
    let Some(next) = add_months_i(start, 1) else {
        return months * MS_MONTH_FALLBACK;
    };
    millis_between(base, start) + frac * millis_between(start, next)
}

// --- format() ---------------------------------------------------------------

const MONTHS_LONG: [&str; 12] = [
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

const DAYS_LONG: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// The tokens, longest first, so `YYYY` wins over `YY` and `LTS` over `LT`.
const TOKENS: [&str; 51] = [
    "LLLL", "DDDD", "MMMM", "dddd", "GGGG", "gggg", "SSSS", "YYYY", "LTS", "LLL", "MMM", "DDD",
    "SSS", "ddd", "LL", "LT", "YY", "MM", "DD", "Do", "dd", "WW", "ww", "HH", "hh", "kk", "mm",
    "ss", "SS", "ZZ", "L", "Y", "Q", "M", "D", "d", "E", "e", "W", "w", "H", "h", "k", "m", "s",
    "S", "A", "a", "X", "x", "Z",
];

/// `format(format: String)` — "Returns the date formatted as specified by a
/// Moment.js format string."
///
/// The run shows only `YYYY-MM-DD` and `YYYY`, so the full token set is
/// `unprobed`. Supported here: `YYYY YY Y`, `Q`, `MMMM MMM MM M`,
/// `DDDD DDD DD Do D`, `dddd ddd dd d`, `E e`, `WW W` (ISO week) and
/// `ww w gggg GGGG` (week and week-year), `HH H hh h kk k`, `mm m`, `ss s`,
/// `SSSS SSS SS S`, `A a`, `X x`, `ZZ Z`, the localized `L LL LLL LLLL LT LTS`
/// in their English forms, and `[literal]` escapes. Anything else passes
/// through as written.
///
/// `Z` and `ZZ` print `+00:00` and `+0000`: this crate holds a civil time and
/// knows no zone, so they are a placeholder rather than an observation.
pub fn format_moment(date: &DateValue, pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::with_capacity(pattern.len() + 8);
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' {
            // An escape runs to the next `]`; an unclosed one is literal text.
            if let Some(end) = chars[i + 1..].iter().position(|c| *c == ']') {
                out.extend(&chars[i + 1..i + 1 + end]);
                i += end + 2;
                continue;
            }
        }
        let rest = &chars[i..];
        let token = TOKENS
            .iter()
            .find(|t| starts_with(rest, t))
            .copied()
            .unwrap_or("");
        if token.is_empty() {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        match localized(token) {
            Some(expansion) => out.push_str(&format_moment(date, expansion)),
            None => out.push_str(&token_value(date, token)),
        }
        i += token.chars().count();
    }
    out
}

fn starts_with(rest: &[char], token: &str) -> bool {
    rest.len() >= token.chars().count() && rest.iter().zip(token.chars()).all(|(a, b)| *a == b)
}

/// The English forms of Moment's localized formats.
fn localized(token: &str) -> Option<&'static str> {
    Some(match token {
        "LT" => "h:mm A",
        "LTS" => "h:mm:ss A",
        "L" => "MM/DD/YYYY",
        "LL" => "MMMM D, YYYY",
        "LLL" => "MMMM D, YYYY h:mm A",
        "LLLL" => "dddd, MMMM D, YYYY h:mm A",
        _ => return None,
    })
}

fn token_value(date: &DateValue, token: &str) -> String {
    let at = date.at;
    let hour12 = match at.hour() % 12 {
        0 => 12,
        h => h,
    };
    let weekday = at.weekday().num_days_from_sunday();
    match token {
        "YYYY" => format!("{:04}", at.year()),
        "YY" => format!("{:02}", at.year().rem_euclid(100)),
        "Y" => at.year().to_string(),
        "Q" => at.month().div_ceil(3).to_string(),
        "MMMM" => MONTHS_LONG[at.month0() as usize].to_string(),
        "MMM" => MONTHS_LONG[at.month0() as usize][..3].to_string(),
        "MM" => format!("{:02}", at.month()),
        "M" => at.month().to_string(),
        "DDDD" => format!("{:03}", at.ordinal()),
        "DDD" => at.ordinal().to_string(),
        "DD" => format!("{:02}", at.day()),
        "Do" => ordinal(at.day()),
        "D" => at.day().to_string(),
        "dddd" => DAYS_LONG[weekday as usize].to_string(),
        "ddd" => DAYS_LONG[weekday as usize][..3].to_string(),
        "dd" => DAYS_LONG[weekday as usize][..2].to_string(),
        "d" | "e" => weekday.to_string(),
        "E" => at.weekday().number_from_monday().to_string(),
        "WW" => format!("{:02}", at.iso_week().week()),
        "W" => at.iso_week().week().to_string(),
        "GGGG" => format!("{:04}", at.iso_week().year()),
        "ww" => format!("{:02}", locale_week(at.date()).0),
        "w" => locale_week(at.date()).0.to_string(),
        "gggg" => format!("{:04}", locale_week(at.date()).1),
        "HH" => format!("{:02}", at.hour()),
        "H" => at.hour().to_string(),
        "hh" => format!("{hour12:02}"),
        "h" => hour12.to_string(),
        "kk" => format!("{:02}", hour24(at.hour())),
        "k" => hour24(at.hour()).to_string(),
        "mm" => format!("{:02}", at.minute()),
        "m" => at.minute().to_string(),
        "ss" => format!("{:02}", at.second()),
        "s" => at.second().to_string(),
        "SSSS" => format!("{:03}0", at.nanosecond() / 1_000_000),
        "SSS" => format!("{:03}", at.nanosecond() / 1_000_000),
        "SS" => format!("{:02}", at.nanosecond() / 10_000_000),
        "S" => (at.nanosecond() / 100_000_000).to_string(),
        "A" => (if at.hour() < 12 { "AM" } else { "PM" }).to_string(),
        "a" => (if at.hour() < 12 { "am" } else { "pm" }).to_string(),
        "X" => at.and_utc().timestamp().to_string(),
        "x" => at.and_utc().timestamp_millis().to_string(),
        "ZZ" => "+0000".to_string(),
        "Z" => "+00:00".to_string(),
        other => other.to_string(),
    }
}

/// Moment's clock hour, 1 through 24.
fn hour24(hour: u32) -> u32 {
    if hour == 0 {
        24
    } else {
        hour
    }
}

/// Moment's `Do` token for a day of the month: 1st, 2nd, 3rd, 4th…
fn ordinal(day: u32) -> String {
    let suffix = match (day % 10, day % 100) {
        (1, 1 | 21 | 31) => "st",
        (2, 2 | 22) => "nd",
        (3, 3 | 23) => "rd",
        _ => "th",
    };
    format!("{day}{suffix}")
}

/// The English locale's week number and week-year: weeks start on Sunday and
/// week 1 is the week containing January 1.
fn locale_week(date: NaiveDate) -> (u32, i32) {
    let year = date.year();
    if let Some(start) = week_one_start(year + 1) {
        if date >= start {
            let weeks = (date - start).num_days() / 7;
            return ((weeks + 1) as u32, year + 1);
        }
    }
    match week_one_start(year) {
        Some(start) if date >= start => {
            let weeks = (date - start).num_days() / 7;
            ((weeks + 1) as u32, year)
        }
        _ => (1, year),
    }
}

fn week_one_start(year: i32) -> Option<NaiveDate> {
    let jan1 = NaiveDate::from_ymd_opt(year, 1, 1)?;
    let back = i64::from(jan1.weekday().num_days_from_sunday());
    jan1.checked_sub_signed(TimeDelta::try_days(back)?)
}
