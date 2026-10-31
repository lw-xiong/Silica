//! The instant a template is expanded at.
//!
//! The crate is clock-free: the caller passes the moment in, so every test
//! is a fixed date and no dependency is needed for one. The app turns a
//! `SystemTime` into a [`Stamp`] with [`Stamp::from_unix`].

/// A civil date and time, already in the zone it is to be written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub year: i64,
    /// 1-12.
    pub month: u32,
    /// 1-31.
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

impl Stamp {
    /// The civil date `offset` seconds east of UTC at `unix` seconds.
    pub fn from_unix(unix: i64, offset: i64) -> Stamp {
        let local = unix + offset;
        let days = local.div_euclid(86_400);
        let secs = local.rem_euclid(86_400);
        let (year, month, day) = civil_from_days(days);
        Stamp {
            year,
            month,
            day,
            hour: (secs / 3600) as u32,
            minute: ((secs % 3600) / 60) as u32,
            second: (secs % 60) as u32,
        }
    }

    /// Days since the Unix epoch, which is what the weekday counts from.
    pub fn days_from_epoch(&self) -> i64 {
        days_from_civil(self.year, self.month, self.day)
    }

    /// Sunday is 0, the way moment's `d` numbers the days.
    pub fn weekday(&self) -> u32 {
        // 1970-01-01 was a Thursday.
        (self.days_from_epoch() + 4).rem_euclid(7) as u32
    }

    /// 1 on the first of January.
    pub fn day_of_year(&self) -> u32 {
        (self.days_from_epoch() - days_from_civil(self.year, 1, 1) + 1) as u32
    }

    pub fn is_leap_year(&self) -> bool {
        let y = self.year;
        (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
    }
}

/// Howard Hinnant's days-from-civil, which is exact for every proleptic
/// Gregorian date and needs no table.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The dates the probe rendered every token against.
    pub(crate) const PROBE: &[(i64, u32, u32, u32, u32, u32)] = &[
        (2026, 1, 1, 0, 5, 0),
        (2024, 2, 29, 13, 45, 30),
        (2026, 12, 31, 23, 59, 59),
        (2026, 10, 18, 9, 7, 3),
        (2026, 10, 19, 0, 0, 0),
    ];

    fn stamp(ix: usize) -> Stamp {
        let (year, month, day, hour, minute, second) = PROBE[ix];
        Stamp {
            year,
            month,
            day,
            hour,
            minute,
            second,
        }
    }

    #[test]
    fn weekdays_match_the_ones_obsidian_rendered() {
        // moment's `d` for the five probe dates.
        let want = [4, 4, 4, 0, 1];
        for (ix, d) in want.iter().enumerate() {
            assert_eq!(stamp(ix).weekday(), *d, "for {:?}", PROBE[ix]);
        }
    }

    #[test]
    fn days_of_the_year_match() {
        // moment's `DDD`, leap year included.
        let want = [1, 60, 365, 291, 292];
        for (ix, d) in want.iter().enumerate() {
            assert_eq!(stamp(ix).day_of_year(), *d, "for {:?}", PROBE[ix]);
        }
    }

    #[test]
    fn a_civil_date_round_trips_through_the_epoch() {
        for (ix, _) in PROBE.iter().enumerate() {
            let s = stamp(ix);
            let unix =
                s.days_from_epoch() * 86_400 + (s.hour * 3600 + s.minute * 60 + s.second) as i64;
            assert_eq!(Stamp::from_unix(unix, 0), s, "for {:?}", PROBE[ix]);
        }
        // The epoch itself, and a date before it.
        assert_eq!(
            Stamp::from_unix(0, 0),
            Stamp {
                year: 1970,
                month: 1,
                day: 1,
                hour: 0,
                minute: 0,
                second: 0
            }
        );
        assert_eq!(Stamp::from_unix(-1, 0).year, 1969);
    }

    #[test]
    fn an_offset_moves_the_civil_date() {
        // Five past midnight UTC is the previous evening six hours west.
        let utc = Stamp::from_unix(1_767_225_900, 0);
        let west = Stamp::from_unix(1_767_225_900, -6 * 3600);
        assert_eq!((utc.year, utc.month, utc.day, utc.hour), (2026, 1, 1, 0));
        assert_eq!(
            (west.year, west.month, west.day, west.hour),
            (2025, 12, 31, 18)
        );
    }
}
