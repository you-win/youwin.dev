//! Calendar identity: what a date in a URL, a range in a query, and a heading on
//! a page have to agree on.
//!
//! At the crate root beside [`crate::tag`] and [`crate::mood`] for the same
//! reason those are: three layers need the same answer. The router decides
//! `/archive/2026/08` is a real month; `db::archive` turns it into the
//! millisecond range a keyset scan can use; the templates title the page and
//! link back to it. If any two disagreed about what a month *is* — whether it
//! ends at midnight UTC, whether `/archive/2026/8` is the same page — the site
//! would list a post on one page and link to it from another.
//!
//! Everything here is UTC, like every other timestamp on this site.

use time::{Date, Month, PrimitiveDateTime, Time};

/// Years the archive will answer for.
///
/// The floor is the epoch, below which a `created_at` is a corrupt row rather
/// than a date. The ceiling is `time`'s own maximum without the `large-dates`
/// feature — past it `Date::from_calendar_date` fails, and a URL that cannot be
/// turned into a range must 404 rather than silently scan everything.
const YEARS: std::ops::RangeInclusive<i32> = 1970..=9999;

/// A year and a month — what `/archive/2026/08` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct YearMonth {
    pub year: i32,
    pub month: u8,
}

impl YearMonth {
    /// Parses the two path segments, or `None` for anything that is not a real
    /// month.
    ///
    /// Accepts `8` as well as `08`, the way [`crate::tag`] accepts `/t/Rust` for
    /// `/t/rust`: the canonical link on the page is what resolves the duplicate,
    /// and 404ing a hand-typed URL that is unambiguous would be pedantry.
    pub fn parse(year: &str, month: &str) -> Option<Self> {
        let year: i32 = year.parse().ok()?;
        let month: u8 = month.parse().ok()?;

        (YEARS.contains(&year) && (1..=12).contains(&month)).then_some(Self { year, month })
    }

    /// Reads back the `YYYY-MM` that `strftime` groups by, so the index page can
    /// turn its own query results into links.
    pub fn from_key(key: &str) -> Option<Self> {
        let (year, month) = key.split_once('-')?;
        Self::parse(year, month)
    }

    /// `[start, end)` in unix millis — the half-open range a month occupies.
    ///
    /// Half-open rather than an inclusive end because the alternative is
    /// "the last millisecond of the month", which is a value somebody eventually
    /// computes as 23:59:59.000 and loses a post to.
    ///
    /// This exists so the month page is an indexed range scan over
    /// `idx_posts_feed` rather than a `strftime` over every row: a date function
    /// in the predicate is opaque to the query planner, and the whole point of
    /// storing millis is that a range is comparable without one.
    pub fn bounds(self) -> Option<(i64, i64)> {
        let next = if self.month == 12 {
            Self { year: self.year + 1, month: 1 }
        } else {
            Self { year: self.year, month: self.month + 1 }
        };

        // December 9999 has no next month inside `YEARS`, so it has no upper
        // bound and no page. Nobody will notice.
        Some((self.start()?, next.start()?))
    }

    fn start(self) -> Option<i64> {
        let date = Date::from_calendar_date(self.year, month_of(self.month)?, 1).ok()?;
        Some(
            PrimitiveDateTime::new(date, Time::MIDNIGHT)
                .assume_utc()
                .unix_timestamp()
                * 1000,
        )
    }

    /// `2026-08` — how `strftime('%Y-%m', …)` spells it, and how these sort.
    pub fn key(self) -> String {
        format!("{:04}-{:02}", self.year, self.month)
    }

    /// `/archive/2026/08` — always zero-padded, which is what makes it canonical.
    pub fn href(self) -> String {
        format!("/archive/{:04}/{:02}", self.year, self.month)
    }

    /// `August 2026`, for a page heading.
    pub fn label(self) -> String {
        format!("{} {}", month_name(self.month), self.year)
    }

    /// `August`, for a row under a year heading that already says the year.
    pub fn month_label(self) -> &'static str {
        month_name(self.month)
    }
}

/// A day of the year without the year — what `/on/08/09` names.
///
/// The whole point is that it has no year: it is every 9th of August the archive
/// contains, which is the one view a multi-year archive has that a single-year
/// one does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonthDay {
    pub month: u8,
    pub day: u8,
}

impl MonthDay {
    /// Parses the two path segments, rejecting days that are not on the calendar.
    ///
    /// Validated against a leap year, so 29 February is a real day here. It has
    /// posts on it about once every four years and a page that 404s on it would
    /// be wrong in exactly the years it mattered.
    pub fn parse(month: &str, day: &str) -> Option<Self> {
        let month: u8 = month.parse().ok()?;
        let day: u8 = day.parse().ok()?;

        Date::from_calendar_date(2024, month_of(month)?, day).ok()?;
        Some(Self { month, day })
    }

    /// The day of the *year* a timestamp falls on, for grouping.
    pub fn of(millis: i64) -> Option<Self> {
        let date = time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(millis) * 1_000_000)
            .ok()?
            .date();
        Some(Self {
            month: date.month() as u8,
            day: date.day(),
        })
    }

    /// `08-09` — how `strftime('%m-%d', …)` spells it.
    pub fn key(self) -> String {
        format!("{:02}-{:02}", self.month, self.day)
    }

    pub fn href(self) -> String {
        format!("/on/{:02}/{:02}", self.month, self.day)
    }

    /// `9 August` — no year, because that is the point of the page.
    pub fn label(self) -> String {
        format!("{} {}", self.day, month_name(self.month))
    }
}

fn month_of(month: u8) -> Option<Month> {
    Month::try_from(month).ok()
}

/// Spelled out rather than taken from `Month`'s `Display`.
///
/// This is the site's display vocabulary, and it should be greppable and
/// changeable here rather than inherited from a dependency's formatting choices.
fn month_name(month: u8) -> &'static str {
    match month {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        _ => "December",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn months_parse_padded_or_not_and_reject_everything_else() {
        assert_eq!(
            YearMonth::parse("2026", "08"),
            Some(YearMonth { year: 2026, month: 8 }),
        );
        assert_eq!(YearMonth::parse("2026", "8"), YearMonth::parse("2026", "08"));

        // Every one of these is reachable by typing into the URL bar.
        for (year, month) in [
            ("2026", "13"),
            ("2026", "0"),
            ("2026", ""),
            ("2026", "aug"),
            ("1969", "01"),
            ("10000", "01"),
            ("", "01"),
            ("-1", "01"),
        ] {
            assert_eq!(
                YearMonth::parse(year, month),
                None,
                "{year:?}/{month:?} is not a month",
            );
        }
    }

    #[test]
    fn a_months_bounds_are_half_open_and_utc() {
        let august = YearMonth { year: 2026, month: 8 };
        let (start, end) = august.bounds().expect("August 2026 is a real month");

        // 2026-08-01T00:00:00Z and 2026-09-01T00:00:00Z. Both are 20454 days
        // (1970→2026) plus the cumulative day-of-year, times 86_400_000 —
        // written out because a hand-copied epoch constant that is four days off
        // still looks entirely plausible.
        assert_eq!(start, (20_454 + 212) * 86_400_000);
        assert_eq!(end, (20_454 + 243) * 86_400_000);
        assert_eq!(end - start, 31 * 86_400_000, "August has 31 days");

        // The last millisecond of August is inside, the first of September is
        // not. This is the assertion the inclusive-end version fails.
        assert!(end - 1 >= start);
        assert_eq!(
            YearMonth { year: 2026, month: 9 }.bounds().unwrap().0,
            end,
            "one month must start exactly where the last ended, with no gap",
        );
    }

    #[test]
    fn december_rolls_into_the_next_year() {
        let (start, end) = YearMonth { year: 2026, month: 12 }
            .bounds()
            .expect("December 2026");
        assert_eq!(end, YearMonth { year: 2027, month: 1 }.bounds().unwrap().0);
        assert!(end > start);
    }

    #[test]
    fn keys_round_trip_and_sort_chronologically() {
        let month = YearMonth { year: 2026, month: 8 };
        assert_eq!(month.key(), "2026-08");
        assert_eq!(YearMonth::from_key("2026-08"), Some(month));
        assert_eq!(YearMonth::from_key("nonsense"), None);

        // Zero-padding is what makes the lexical sort a chronological one, which
        // is what the index page's ORDER BY relies on.
        assert!(YearMonth { year: 2026, month: 9 }.key() > month.key());
        assert!(YearMonth { year: 2026, month: 10 }.key() > YearMonth { year: 2026, month: 9 }.key());
    }

    #[test]
    fn hrefs_are_always_padded() {
        assert_eq!(YearMonth { year: 2026, month: 8 }.href(), "/archive/2026/08");
        assert_eq!(MonthDay { month: 8, day: 9 }.href(), "/on/08/09");
    }

    #[test]
    fn the_leap_day_is_a_real_day_and_the_impossible_ones_are_not() {
        assert!(MonthDay::parse("02", "29").is_some());

        for (month, day) in [
            ("02", "30"),
            ("04", "31"),
            ("13", "01"),
            ("00", "01"),
            ("01", "00"),
            ("01", "32"),
            ("1", "x"),
        ] {
            assert_eq!(MonthDay::parse(month, day), None, "{month}/{day}");
        }
    }

    #[test]
    fn a_timestamp_knows_what_day_of_the_year_it_is() {
        // 2026-08-09T07:06:39Z, the same sample time_fmt uses.
        assert_eq!(
            MonthDay::of(1_786_259_199_000),
            Some(MonthDay { month: 8, day: 9 }),
        );
        assert_eq!(MonthDay::of(1_786_259_199_000).unwrap().key(), "08-09");
    }

    #[test]
    fn labels_read_as_prose() {
        assert_eq!(YearMonth { year: 2026, month: 8 }.label(), "August 2026");
        assert_eq!(MonthDay { month: 8, day: 9 }.label(), "9 August");
        assert_eq!(MonthDay { month: 3, day: 1 }.label(), "1 March");
    }
}
