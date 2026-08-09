//! The archive's own rhythm, measured in the units the writer actually works in.
//!
//! Everything about the pet that involves waiting — how fast energy decays, how
//! much one new post is worth — has to be judged against how often this
//! particular person writes. Measuring that badly is not a cosmetic problem: it
//! decides whether an ordinary Wednesday reads as a normal quiet stretch or as
//! abandonment.
//!
//! Two ideas do the work.
//!
//! **Sittings, not posts.** People do not write on a schedule; they sit down.
//! Five notes in ten minutes is one sitting, and treating the gaps *between*
//! those notes as the writer's rhythm is what made the first version unusable:
//! someone who wrote five posts every Sunday measured a cadence of a few minutes,
//! got a decay half-life pinned to its two-hour floor, and had a pet lying flat
//! from Monday morning to the following weekend — while doing exactly what they
//! had always done. Sessions are the unit because sessions are the behaviour.
//!
//! **Order statistics, not moments.** Gaps between sittings span orders of
//! magnitude and lean hard to the right: a fortnight away is one number among a
//! hundred ordinary evenings, and a mean is hostage to it. Quantiles are not, and
//! they are read straight off the sorted sample without assuming the gaps are
//! distributed in any particular shape — which they are not.
//!
//! This did start out as a median and a spread taken in log space, on the theory
//! that log-gaps are roughly normal. Two things killed it. Quantiles are
//! invariant under any monotone transform, so the logarithm changed nothing about
//! the two numbers below; and the median absolute deviation, the robust spread it
//! used, collapses to exactly zero once more than half the gaps are identical —
//! which is the common case of somebody who writes most days and occasionally
//! disappears for a week. That writer would have been handed a spread of nothing
//! and no tolerance at all, which is precisely backwards.
//!
//! Nothing here is stored, and nothing here needs `now`: a rhythm is a property
//! of the posts, not of the moment they are being read at.

use crate::familiar::Morsel;

/// The silence that ends a sitting.
///
/// Deliberately a little wider than [`super::energy`]'s 30-minute burst window:
/// two posts close enough to stack into one burst must never be counted as two
/// separate visits to the composer. Past three quarters of an hour you got up and
/// did something else.
const SESSION_GAP_MINUTES: f64 = 45.0;

/// How many of the most recent gaps between sittings the rhythm is read from.
///
/// A count rather than a time window, which is what makes it self-scaling: for
/// someone who writes hourly this is most of a day and a change of habit shows up
/// within one, and for someone who writes weekly it is four months of habit. No
/// window measured in days can do both — seven days holds a single gap for the
/// weekly writer, which is not a distribution at all, and that failure is the
/// whole reason this module exists.
const GAP_SAMPLE: usize = 16;

/// The rhythm assumed for an archive with nothing to measure yet: one sitting, or
/// none. Six hours is a working day with a couple of visits in it.
pub const FALLBACK_GAP_HOURS: f64 = 6.0;

/// How often this archive is written, and how regularly.
///
/// Two points on the writer's own distribution of gaps, which between them answer
/// both questions any waiting calculation needs to ask: *how long is a normal gap
/// for this person*, and *how long is a long one*.
///
/// `Copy`, like [`super::PetState`]: it is two floats, and there is no reason for
/// a caller to think about ownership of a measurement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Baseline {
    typical_hours: f64,
    long_hours: f64,
}

impl Default for Baseline {
    fn default() -> Self {
        Self {
            typical_hours: FALLBACK_GAP_HOURS,
            long_hours: FALLBACK_GAP_HOURS,
        }
    }
}

impl Baseline {
    /// Reads the rhythm out of an archive. `posts` must be sorted oldest first,
    /// which is how `db::familiar::all` returns them.
    ///
    /// An archive with fewer than two sittings has no gap to measure and falls
    /// back rather than inventing one — the same thing the pet does on its first
    /// day, and it is corrected by the second sitting.
    pub fn of(posts: &[Morsel]) -> Self {
        let gaps = session_gap_hours(posts);
        let mut recent: Vec<f64> = gaps[gaps.len().saturating_sub(GAP_SAMPLE)..].to_vec();

        if recent.is_empty() {
            return Self::default();
        }
        recent.sort_by(f64::total_cmp);

        Self {
            typical_hours: quantile(&recent, 0.50),
            long_hours: quantile(&recent, 0.75),
        }
    }

    /// The gap between sittings this writer is usually working to.
    ///
    /// The middle of their own distribution rather than an average of it, so one
    /// sleepless fortnight does not move it.
    pub fn typical_gap_hours(self) -> f64 {
        self.typical_hours
    }

    /// The gap this writer exceeds about a quarter of the time.
    ///
    /// The number that decides how long the pet waits before it visibly sags,
    /// because it already contains both halves of the question. A metronome's
    /// quartiles sit on top of each other, so its long gap *is* its typical one
    /// and anything further is immediately out of character. Somebody whose weeks
    /// vary has them far apart, so a stretch that would alarm the metronome is,
    /// for them, a Tuesday.
    ///
    /// One number covers three orders of magnitude of posting habit, which is why
    /// it replaced a hand-picked multiple of the mean.
    pub fn long_gap_hours(self) -> f64 {
        self.long_hours
    }
}

/// Hours between the starts of consecutive sittings.
///
/// A post starts a new sitting when more than [`SESSION_GAP_MINUTES`] separates
/// it from the one before. Every gap this returns is therefore larger than that
/// threshold and comfortably positive, which is what makes the logarithms above
/// safe — including for two posts that share a timestamp, which are one sitting
/// by the same rule.
fn session_gap_hours(posts: &[Morsel]) -> Vec<f64> {
    let mut starts: Vec<i64> = Vec::new();
    let mut previous: Option<i64> = None;

    for post in posts {
        let at = post.created_at;
        if previous.is_none_or(|before| minutes(at - before) > SESSION_GAP_MINUTES) {
            starts.push(at);
        }
        previous = Some(at);
    }

    starts
        .windows(2)
        .map(|pair| hours(pair[1] - pair[0]))
        .collect()
}

/// The `p` quantile of an already-sorted, non-empty sample, interpolating
/// linearly between the two order statistics it falls between.
///
/// The convention R and NumPy both default to. With sixteen gaps at most, which
/// definition of "the 75th percentile" is chosen actually moves the number, and
/// picking the common one means it can be checked against anything else.
fn quantile(sorted: &[f64], p: f64) -> f64 {
    let position = p * (sorted.len() - 1) as f64;
    let below = position.floor() as usize;
    let above = position.ceil() as usize;

    sorted[below] + (sorted[above] - sorted[below]) * (position - below as f64)
}

fn hours(millis: i64) -> f64 {
    millis as f64 / 3_600_000.0
}

fn minutes(millis: i64) -> f64 {
    millis as f64 / 60_000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::familiar::fixture::{DAY, HOUR, START, post, run};

    const MINUTE: i64 = 60_000;

    /// `count` posts five minutes apart — one sitting, however many notes.
    fn sitting(from: i64, count: usize) -> Vec<Morsel> {
        (0..count)
            .map(|i| post(from + i as i64 * 5 * MINUTE, "another note"))
            .collect()
    }

    /// `weeks` Sunday sittings of five posts each. The writer the old mean-gap
    /// cadence could not see.
    fn weekly_bursts(weeks: usize) -> Vec<Morsel> {
        (0..weeks)
            .flat_map(|week| sitting(START + week as i64 * 7 * DAY + 10 * HOUR, 5))
            .collect()
    }

    #[test]
    fn a_handful_of_notes_in_one_go_is_a_single_sitting() {
        assert!(session_gap_hours(&sitting(START, 5)).is_empty());

        // And the threshold is where it says it is: 45 minutes apart is still one
        // sitting, 46 is two.
        let just_inside = [post(START, "a"), post(START + 45 * MINUTE, "b")];
        assert!(session_gap_hours(&just_inside).is_empty());

        let just_outside = [post(START, "a"), post(START + 46 * MINUTE, "b")];
        assert_eq!(session_gap_hours(&just_outside).len(), 1);
    }

    #[test]
    fn gaps_are_measured_between_the_starts_of_sittings() {
        // Two sittings a day apart, each with several posts in it. The rhythm is
        // the day, not the five minutes inside either one.
        let mut posts = sitting(START, 4);
        posts.extend(sitting(START + DAY, 4));

        let gaps = session_gap_hours(&posts);
        assert_eq!(gaps.len(), 1);
        assert!((gaps[0] - 24.0).abs() < 1e-9, "{gaps:?}");
    }

    #[test]
    fn a_weekly_burst_writer_has_a_weekly_rhythm() {
        // The bug this module exists for. Under the mean gap between *posts* this
        // archive measured a cadence of five minutes; it is plainly a week.
        let rhythm = Baseline::of(&weekly_bursts(8));

        assert!(
            (rhythm.typical_gap_hours() - 168.0).abs() < 1e-6,
            "{} hours",
            rhythm.typical_gap_hours(),
        );
    }

    #[test]
    fn an_hourly_writer_has_an_hourly_rhythm() {
        // The other end of the range, through the same formula and constants.
        let rhythm = Baseline::of(&run(START, 12, "a note"));
        assert!((rhythm.typical_gap_hours() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn regularity_decides_how_long_a_long_gap_is() {
        // A metronome: every sitting exactly a day apart. Nothing is forgiven
        // beyond the rhythm itself, because nothing ever varied from it.
        let metronome: Vec<_> = (0..12)
            .flat_map(|day| sitting(START + day * DAY, 2))
            .collect();
        let steady = Baseline::of(&metronome);
        assert!((steady.long_gap_hours() - steady.typical_gap_hours()).abs() < 1e-9);

        // Someone with the same typical gap who writes most days and now and then
        // disappears for a week. Their quarter-of-the-time gap is much longer, so
        // the pet waits longer before it reads silence as anything.
        //
        // This is also the shape that broke the median absolute deviation the
        // first version used: seven of these eleven gaps are identical, which is
        // over half, which drove a robust spread to exactly zero and left the
        // least regular writer with the least tolerance.
        let scattered: Vec<_> = [0, 1, 4, 5, 6, 12, 13, 20, 21, 22, 30, 31]
            .into_iter()
            .flat_map(|day| sitting(START + day * DAY, 2))
            .collect();
        let irregular = Baseline::of(&scattered);

        assert_eq!(irregular.typical_gap_hours(), steady.typical_gap_hours());
        assert!(
            irregular.long_gap_hours() > steady.long_gap_hours(),
            "irregular {} should be forgiven longer than steady {}",
            irregular.long_gap_hours(),
            steady.long_gap_hours(),
        );
    }

    #[test]
    fn the_rhythm_follows_the_most_recent_sittings() {
        // A dense past and a slower present. Once the recent sample is entirely
        // the new habit, that is the rhythm — the old one is history.
        let mut posts = run(START, 40, "the old fast habit");
        let switched = START + 40 * HOUR;
        posts.extend(
            (0..20).map(|day| post(switched + day * DAY, "the new slow habit")),
        );

        let rhythm = Baseline::of(&posts);
        assert!((rhythm.typical_gap_hours() - 24.0).abs() < 1e-9, "{}", rhythm.typical_gap_hours());
    }

    #[test]
    fn a_single_enormous_gap_does_not_become_the_rhythm() {
        // A year off, then business as usual. A mean would still be talking about
        // the year in six months' time; the median has already forgotten it.
        let mut posts = vec![post(START, "before")];
        posts.extend(run(START + 365 * DAY, 12, "after"));

        let rhythm = Baseline::of(&posts);
        assert!((rhythm.typical_gap_hours() - 1.0).abs() < 1e-9, "{}", rhythm.typical_gap_hours());
    }

    #[test]
    fn nothing_to_measure_falls_back_rather_than_inventing_a_number() {
        for archive in [vec![], vec![post(START, "the very first")], sitting(START, 6)] {
            let rhythm = Baseline::of(&archive);
            assert_eq!(rhythm.typical_gap_hours(), FALLBACK_GAP_HOURS);
            assert_eq!(rhythm.long_gap_hours(), FALLBACK_GAP_HOURS);
        }
    }

    #[test]
    fn the_quantile_interpolates_between_order_statistics() {
        assert_eq!(quantile(&[1.0, 2.0, 3.0], 0.5), 2.0);
        assert_eq!(quantile(&[1.0, 2.0, 3.0, 4.0], 0.5), 2.5);
        assert_eq!(quantile(&[1.0, 2.0, 3.0, 4.0, 5.0], 0.75), 4.0);

        // The ends, and a sample of one.
        assert_eq!(quantile(&[1.0, 2.0, 3.0], 0.0), 1.0);
        assert_eq!(quantile(&[1.0, 2.0, 3.0], 1.0), 3.0);
        assert_eq!(quantile(&[7.0], 0.75), 7.0);
    }
}
