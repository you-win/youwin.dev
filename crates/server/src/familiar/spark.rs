//! Things that happened recently enough to still be showing.
//!
//! The pet's first **transient**, and the shape is worth getting right once
//! because anticipation and appetite will both want it. Everything else in
//! [`PetState`](super::PetState) is a steady-state function of the present: what
//! the archive looks like now, drawn now. A spark is not — it is an event that
//! happened at a moment and fades afterwards.
//!
//! It still holds the invariant that keeps the pet schema-free. The event's
//! timestamp is in the archive, how long ago it was comes from `now`, and the
//! window it survives comes from the writer's own rhythm — so this remains a pure
//! function of `(posts, now)` with nothing stored and nothing to fall out of step
//! with the posts. Delete the post that caused it and it never happened.
//!
//! Two events, and one of them exists to fix the other.
//!
//! **Milestones already existed and were unwatchable.** The pose was drawn while
//! the post count was *exactly* ten, fifty or a hundred — a window of zero. Post
//! your fiftieth and fifty-first an hour apart and the only celebration the pet
//! ever offers existed for that hour, whether or not anybody loaded the page. A
//! reward you can miss without knowing it was there is worse than none.
//!
//! **Rekindling is the one the whole thing was missing.** Coming back after a
//! real absence showed a floored pet climbing slowly out of it: the absence was
//! punished and the return was not rewarded, which is backwards for a creature
//! meant to be a reason to write again. The first sitting after a silence longer
//! than nine tenths of this writer's own is now an event in its own right.

use crate::familiar::{Baseline, Morsel};

/// Post counts worth a small fuss. Round numbers, and rare enough that each one
/// happens at most once.
const MILESTONES: [usize; 3] = [10, 50, 100];

/// Gaps needed before "longer than nine tenths of yours" is a statement rather
/// than an opinion.
///
/// Half a sample. Below it the quantile is being read off a handful of numbers,
/// and a blog on its third sitting would announce a triumphant return from an
/// afternoon. Not the full sample that [`super::speech`]'s odd-hour claim
/// demands, because that one is judged against a *prior* and this one is judged
/// against observations, however few — and because a fortnight of silence is
/// exactly when somebody most needs welcoming back, not when they need to be
/// told there is insufficient data.
const ENOUGH_GAPS: usize = 8;

/// How many of the writer's own gaps a spark survives.
///
/// Two rather than one, and the difference matters: at one, a daily writer's
/// milestone expires at the exact moment they next sit down to write, so they
/// would miss by a hair the one thing it was held open for. A spark has to last
/// *past* the next sitting, not up to it.
const WINDOW_GAPS: f64 = 2.0;

/// The bounds on that window.
///
/// A day at the bottom, because an hourly writer's rhythm would otherwise let a
/// lifetime event pass in two hours. A fortnight at the top, which is the same
/// fortnight [`super::energy`] calls the pet's memory.
const MIN_WINDOW_HOURS: f64 = 24.0;
const MAX_WINDOW_HOURS: f64 = 14.0 * 24.0;

/// How many typical gaps a silence has to run before it is an absence.
///
/// The quantile alone is not enough, and the reason is the same degeneracy that
/// ruled out a median absolute deviation in [`super::baseline`]: for a writer
/// whose gaps are all alike, the ninetieth percentile *is* the ordinary gap, so
/// anything at all beyond it clears the bar. A daily writer skipping one day
/// would have been welcomed back from nowhere. An absence has to be both unusual
/// for this writer and long in its own right, and this is the second half.
const ABSENCE_MULTIPLE: f64 = 3.0;

const HOUR_MILLIS: i64 = 3_600_000;

/// What the archive has done recently enough that the pet is still showing it.
///
/// Both at once is an ordinary case — a return that happens to be the fiftieth
/// post — and **both are reported**. Choosing between them is a *drawing*
/// decision, because there is one pair of corners and only one thing can go in
/// it, and that decision belongs to the renderer.
///
/// It used to be made here, and putting it here was a mistake worth recording.
/// Returning one winner meant a rekindling that coincided with a milestone
/// simply did not exist, and [`super::speech`] gates its return line on this —
/// so the pet drew a `*` and had nothing whatsoever to say about the three weeks
/// it had just been away, which was the most surprising fact available to it. A
/// constraint on the corners had quietly become a constraint on what the pet was
/// allowed to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sparks {
    /// The round number of posts just crossed, if one was.
    pub milestone: Option<usize>,
    /// Came back from a silence longer than this writer usually keeps.
    pub rekindled: bool,
}

/// Everything the archive has done recently enough to still be showing.
///
/// `posts` must be sorted oldest first and already filtered to what is visible at
/// `now`, which is what [`super::compute`] hands it.
pub fn detect(posts: &[Morsel], rhythm: &Baseline, now: i64) -> Sparks {
    let window = window_hours(rhythm);

    Sparks {
        milestone: MILESTONES.into_iter().rev().find(|count| {
            posts.len() >= *count && within(posts[count - 1].created_at, window, now)
        }),
        rekindled: rekindled(rhythm, window, now),
    }
}

/// Whether the latest sitting ended a real absence, and is still recent.
fn rekindled(rhythm: &Baseline, window: f64, now: i64) -> bool {
    if rhythm.measured_gaps() < ENOUGH_GAPS {
        return false;
    }

    let (Some(start), Some(gap)) = (
        rhythm.latest_sitting_start(),
        rhythm.gap_before_latest_sitting(),
    ) else {
        return false;
    };

    within(start, window, now)
        && gap > rhythm.rare_gap_hours()
        && gap > rhythm.typical_gap_hours() * ABSENCE_MULTIPLE
}

fn window_hours(rhythm: &Baseline) -> f64 {
    (rhythm.typical_gap_hours() * WINDOW_GAPS).clamp(MIN_WINDOW_HOURS, MAX_WINDOW_HOURS)
}

/// Whether `at` is in the past and still inside `window` hours of `now`.
///
/// Half-open from zero, like the snapshot's own freshness check and for the same
/// reason: a clock that has gone backwards should read as "not yet" rather than
/// as an event from the far future that never expires.
fn within(at: i64, window: f64, now: i64) -> bool {
    let since = (now - at) as f64 / HOUR_MILLIS as f64;
    (0.0..window).contains(&since)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::familiar::fixture::{DAY, HOUR, START, post, run};

    const MINUTE: i64 = 60_000;

    /// A daily writer, `days` of them, one note each.
    fn daily(days: i64) -> Vec<Morsel> {
        (0..days).map(|day| post(START + day * DAY, "a note")).collect()
    }

    fn detect_at(posts: &[Morsel], now: i64) -> Sparks {
        detect(posts, &Baseline::of(posts), now)
    }

    fn milestone(count: usize) -> Sparks {
        Sparks {
            milestone: Some(count),
            rekindled: false,
        }
    }

    const REKINDLED: Sparks = Sparks {
        milestone: None,
        rekindled: true,
    };
    const NOTHING: Sparks = Sparks {
        milestone: None,
        rekindled: false,
    };

    #[test]
    fn a_milestone_outlives_the_post_that_caused_it() {
        // The bug. Under the old rule this was drawn only while the count was
        // exactly fifty, so a fifty-first post an hour later ended the only
        // celebration the pet offers — seen or not.
        let posts = daily(51);
        let fiftieth = START + 49 * DAY;

        assert_eq!(detect_at(&posts[..50], fiftieth), milestone(50));

        // Still showing many hours after the fifty-first landed.
        let after = START + 50 * DAY + HOUR;
        assert_eq!(detect_at(&posts, after), milestone(50));
    }

    #[test]
    fn a_milestone_does_expire() {
        let posts = daily(50);
        // A daily writer's window is a day, and the fiftieth post was two ago.
        assert_eq!(detect_at(&posts, START + 51 * DAY), NOTHING);
    }

    #[test]
    fn the_window_follows_the_writers_own_rhythm() {
        // An hourly writer would lose a lifetime event in sixty minutes, so the
        // floor holds it for a day.
        let hourly = run(START, 50, "a note");
        let fiftieth = START + 49 * HOUR;
        assert_eq!(
            detect_at(&hourly, fiftieth + 12 * HOUR),
            milestone(50),
        );
        assert_eq!(detect_at(&hourly, fiftieth + 25 * HOUR), NOTHING);

        // A weekly writer still sees it when they next sit down, which is the
        // whole point of not fixing the window at a day.
        let weekly: Vec<_> = (0..50).map(|week| post(START + week * 7 * DAY, "a note")).collect();
        let fiftieth = START + 49 * 7 * DAY;
        assert_eq!(
            detect_at(&weekly, fiftieth + 6 * DAY),
            milestone(50),
        );
    }

    #[test]
    fn the_latest_milestone_wins_when_two_land_together() {
        // Forty notes in an afternoon takes an archive of nine past both ten and
        // fifty inside one window. The bigger number is the one worth showing.
        let mut posts = daily(9);
        let spree = START + 9 * DAY;
        posts.extend((0..45).map(|i| post(spree + i * MINUTE, "and another")));

        assert_eq!(
            detect_at(&posts, spree + 46 * MINUTE),
            milestone(50),
        );
    }

    #[test]
    fn coming_back_from_a_real_absence_is_an_event() {
        // Twenty days of a daily note, three weeks of nothing, then a return.
        let mut posts = daily(20);
        let back = START + 19 * DAY + 21 * DAY;
        posts.push(post(back, "back at it"));

        assert_eq!(detect_at(&posts, back + HOUR), REKINDLED);

        // And the rest of that sitting does not re-trigger it — the sitting is
        // the unit, so five notes on the way back are one return.
        posts.extend((1..5).map(|i| post(back + i * MINUTE, "and another")));
        assert_eq!(detect_at(&posts, back + 6 * MINUTE), REKINDLED);
    }

    #[test]
    fn an_ordinary_gap_is_not_a_return() {
        // A daily writer skipping one day has not been anywhere. This is the case
        // the quantile could not judge on its own: fifteen of their sixteen gaps
        // are the same length, so their ninetieth percentile *is* an ordinary day
        // and two days clears it. Being three times the usual gap is the test it
        // fails.
        let mut posts = daily(20);
        let next = START + 19 * DAY + 2 * DAY;
        posts.push(post(next, "morning"));

        assert_eq!(detect_at(&posts, next + HOUR), NOTHING);

        // Four days away, on the other hand, is a real absence for them.
        let mut posts = daily(20);
        let later = START + 19 * DAY + 4 * DAY;
        posts.push(post(later, "back"));

        assert_eq!(detect_at(&posts, later + HOUR), REKINDLED);
    }

    #[test]
    fn a_writer_who_always_vanishes_is_never_coming_back_from_anywhere() {
        // Somebody whose rhythm *is* a fortnight on and off. A three-week gap is
        // inside their own habit, so it is not an absence to be welcomed back
        // from — the quantile is doing the work a fixed threshold could not.
        // A pair of notes, then nineteen days off, over and over — and the run
        // ends on one of those nineteen-day gaps, which is the one being judged.
        let erratic: Vec<_> = [0, 1, 20, 21, 40, 41, 60, 61, 80, 81, 100, 101, 120, 121, 140]
            .into_iter()
            .map(|day| post(START + day * DAY, "a note"))
            .collect();
        let last = erratic.last().expect("posts").created_at;

        assert_eq!(detect_at(&erratic, last + HOUR), NOTHING);
    }

    #[test]
    fn nothing_is_claimed_from_too_few_gaps() {
        // A blog on its third sitting has not returned triumphantly from an
        // afternoon, however that afternoon compares to its two predecessors.
        let posts = [
            post(START, "one"),
            post(START + HOUR, "two"),
            post(START + 12 * HOUR, "three"),
        ];
        assert_eq!(detect_at(&posts, START + 13 * HOUR), NOTHING);
    }

    #[test]
    fn both_are_reported_when_both_happened() {
        // Coming back after three weeks, and it happens to be the fiftieth post.
        // Both are true, and this used to report only the milestone — which meant
        // the pet had no idea it had been away, because `speech` reads this and
        // there was nothing here to read. Choosing between them is the renderer's
        // problem: there is one pair of corners, and speech is not drawn in them.
        let mut posts = daily(49);
        let back = START + 48 * DAY + 21 * DAY;
        posts.push(post(back, "back, and it is the fiftieth"));

        assert_eq!(
            detect_at(&posts, back + HOUR),
            Sparks {
                milestone: Some(50),
                rekindled: true,
            },
        );
    }

    #[test]
    fn an_empty_archive_sparks_at_nothing() {
        assert_eq!(detect_at(&[], START), NOTHING);
        assert_eq!(detect_at(&[post(START, "the first")], START + HOUR), NOTHING);
    }

    #[test]
    fn a_clock_that_jumped_backwards_is_not_a_permanent_celebration() {
        let posts = daily(50);
        let fiftieth = START + 49 * DAY;

        // Read from before the post that caused it: not yet, rather than an
        // event from the future that never expires.
        assert_eq!(detect_at(&posts, fiftieth - HOUR), NOTHING);
    }
}
