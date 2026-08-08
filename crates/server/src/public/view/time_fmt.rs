//! Timestamps are stored as unix millis UTC and rendered in UTC.
//!
//! No local-time handling anywhere: the server has no business guessing a
//! reader's zone, and a fixed zone keeps the edge cache from having to vary.

use time::{OffsetDateTime, format_description::BorrowedFormatItem, macros::format_description};

const HUMAN: &[BorrowedFormatItem<'_>] = format_description!("[day padding:none] [month repr:short] [year]");
const HUMAN_WITH_TIME: &[BorrowedFormatItem<'_>] =
    format_description!("[day padding:none] [month repr:short] [year] · [hour]:[minute] UTC");

fn to_datetime(millis: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(millis) * 1_000_000)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
}

/// "8 Aug 2026" — for feed rows.
pub fn human(millis: i64) -> String {
    to_datetime(millis)
        .format(HUMAN)
        .unwrap_or_else(|_| String::new())
}

/// "8 Aug 2026 · 21:14 UTC" — for permalinks, where precision is wanted.
pub fn human_with_time(millis: i64) -> String {
    to_datetime(millis)
        .format(HUMAN_WITH_TIME)
        .unwrap_or_else(|_| String::new())
}

/// RFC 3339, for `<time datetime>`, `article:published_time`, and Atom.
pub fn rfc3339(millis: i64) -> String {
    to_datetime(millis)
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| String::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2026-08-09T07:06:39Z
    const SAMPLE: i64 = 1_786_259_199_000;

    #[test]
    fn formats_are_stable_and_utc() {
        assert_eq!(human(SAMPLE), "9 Aug 2026");
        assert_eq!(human_with_time(SAMPLE), "9 Aug 2026 · 07:06 UTC");
        assert_eq!(rfc3339(SAMPLE), "2026-08-09T07:06:39Z");
    }

    #[test]
    fn absurd_timestamps_degrade_instead_of_panicking() {
        // A bad row must not take the whole feed down with it.
        assert_eq!(human(i64::MAX), "1 Jan 1970");
        assert_eq!(rfc3339(i64::MIN), "1970-01-01T00:00:00Z");
    }
}
