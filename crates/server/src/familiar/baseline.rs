//! What this writer's own habits look like, measured in the units they work in.
//!
//! Everything about the pet that compares the present to the past — how fast
//! energy decays, how much one new post is worth, whether anything that just
//! happened is worth remarking on — has to be judged against this particular
//! person. Eight hours is an ordinary afternoon for one archive and a
//! disappearance for another; four hundred words is a monologue from someone who
//! writes in fragments and a Tuesday from someone who does not.
//!
//! Three distributions, all built the same way. **Gaps** between sittings set the
//! decay curve. **Sitting sizes** say whether a run of posts is a burst. **Words
//! per post** say whether something was long. [`speech`](super::speech) reads all
//! three; [`energy`](super::energy) and [`stats`](super::stats) read the first.
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
//! **Order statistics, not moments.** These quantities span orders of magnitude
//! and lean hard to the right: a fortnight away is one number among a hundred
//! ordinary evenings, and a mean is hostage to it. Quantiles are not, and they
//! are read straight off the sorted sample without assuming any particular shape.
//!
//! This did start out as a median and a spread taken in log space, on the theory
//! that log-gaps are roughly normal. Two things killed it. Quantiles are
//! invariant under any monotone transform, so the logarithm changed nothing; and
//! the median absolute deviation, the robust spread it used, collapses to exactly
//! zero once more than half the values are identical — the common case of
//! somebody who writes most days and disappears for a week now and then. That
//! writer would have been handed no tolerance at all, which is backwards.
//!
//! Nothing here is stored, and nothing here needs `now`: a habit is a property of
//! the posts, not of the moment they are being read at.

use crate::familiar::Morsel;

/// The silence that ends a sitting.
///
/// Deliberately a little wider than [`super::energy`]'s 30-minute burst window:
/// two posts close enough to stack into one burst must never be counted as two
/// separate visits to the composer. Past three quarters of an hour you got up and
/// did something else.
const SESSION_GAP_MINUTES: f64 = 45.0;

/// How many of the most recent observations each distribution is read from.
///
/// A count rather than a time window, which is what makes it self-scaling: for
/// someone who writes hourly, sixteen gaps is most of a day and a change of habit
/// shows up within one; for someone who writes weekly it is four months of habit.
/// No window measured in days can do both — seven days holds a single gap for the
/// weekly writer, which is not a distribution at all, and that failure is the
/// whole reason this module exists.
///
/// It is also the resolution limit on how surprised anything downstream is
/// allowed to be, which is why [`super::speech`] can see it. See
/// [`Sample::at_least`].
pub const SAMPLE: usize = 16;

/// The rhythm assumed for an archive with nothing to measure yet: one sitting, or
/// none. Six hours is a working day with a couple of visits in it.
pub const FALLBACK_GAP_HOURS: f64 = 6.0;

/// The gap the writer exceeds about a quarter of the time. Sets the decay curve.
const UPPER_QUARTILE: f64 = 0.75;

/// The gap the writer exceeds about one time in ten. A silence past this is an
/// absence rather than a quiet stretch, which is what [`super::spark`] welcomes
/// them back from.
const RARE: f64 = 0.90;

/// One distribution, as its most recent [`SAMPLE`] observations, sorted.
///
/// A fixed array rather than a `Vec` so the whole [`Baseline`] is one flat value
/// with no allocation behind it, which is what lets it be built cheaply inside
/// three separate callers rather than threaded through their signatures.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Sample {
    values: [f64; SAMPLE],
    len: usize,
}

impl Sample {
    /// The most recent `SAMPLE` of `values`, in the order they happened.
    fn of(values: &[f64]) -> Self {
        let recent = &values[values.len().saturating_sub(SAMPLE)..];

        let mut sample = Self {
            values: [0.0; SAMPLE],
            len: recent.len(),
        };
        sample.values[..recent.len()].copy_from_slice(recent);
        sample.values[..recent.len()].sort_by(f64::total_cmp);
        sample
    }

    fn is_empty(self) -> bool {
        self.len == 0
    }

    /// The `p` quantile, interpolating linearly between the two order statistics
    /// it falls between. `None` for an empty sample.
    ///
    /// The convention R and NumPy both default to. With sixteen observations at
    /// most, which definition of "the 75th percentile" is chosen actually moves
    /// the number, and picking the common one means it can be checked against
    /// anything else.
    fn quantile(self, p: f64) -> Option<f64> {
        if self.is_empty() {
            return None;
        }

        let position = p * (self.len - 1) as f64;
        let below = position.floor() as usize;
        let above = position.ceil() as usize;

        Some(self.values[below] + (self.values[above] - self.values[below]) * (position - below as f64))
    }

    /// The probability that an observation from this writer's history is at least
    /// `value` — the tail this sample says the present sits in.
    ///
    /// Laplace-smoothed, `(count + 1) / (n + 1)`, which matters more than it
    /// looks. An unsmoothed count of zero says "this has never happened and never
    /// could", and `-log2(0)` is infinite: one unprecedented post would out-shout
    /// everything the pet could ever say again. Sixteen observations cannot tell
    /// one-in-twenty from one-in-ever, and the smoothing is that admission — the
    /// rarest thing this can report is a shade under one in seventeen.
    ///
    /// An empty sample has nothing to be surprised by, so everything is ordinary.
    fn at_least(self, value: f64) -> f64 {
        if self.is_empty() {
            return 1.0;
        }

        let count = self.values[..self.len].iter().filter(|held| **held >= value).count();
        (count as f64 + 1.0) / (self.len as f64 + 1.0)
    }
}

/// How often this archive is written, how much lands each time, and how long the
/// pieces are.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Baseline {
    gaps: Sample,
    sittings: Sample,
    words: Sample,
    /// The most recent sitting — the observation the samples above exist to
    /// judge, rather than another distribution. Held here so the clustering runs
    /// once for both.
    latest_sitting: usize,
    latest_start: Option<i64>,
    /// Silence before the latest sitting. `None` when it is the only one.
    latest_gap: Option<f64>,
}

impl Default for Baseline {
    fn default() -> Self {
        Self {
            gaps: Sample::of(&[]),
            sittings: Sample::of(&[]),
            words: Sample::of(&[]),
            latest_sitting: 0,
            latest_start: None,
            latest_gap: None,
        }
    }
}

impl Baseline {
    /// Reads the habits out of an archive. `posts` must be sorted oldest first,
    /// which is how `db::familiar::all` returns them.
    ///
    /// An archive with fewer than two sittings has no gap to measure and falls
    /// back rather than inventing one — the same thing the pet does on its first
    /// day, and it is corrected by the second sitting.
    pub fn of(posts: &[Morsel]) -> Self {
        let sittings = sittings(posts);

        let gaps: Vec<f64> = sittings
            .windows(2)
            .map(|pair| hours(pair[1].0 - pair[0].0))
            .collect();
        let sizes: Vec<f64> = sittings.iter().map(|(_, posts)| *posts as f64).collect();
        let words: Vec<f64> = posts.iter().map(|post| words(post) as f64).collect();

        Self {
            latest_sitting: sittings.last().map_or(0, |(_, posts)| *posts),
            latest_start: sittings.last().map(|(start, _)| *start),
            latest_gap: gaps.last().copied(),
            gaps: Sample::of(&gaps),
            sittings: Sample::of(&sizes),
            words: Sample::of(&words),
        }
    }

    /// The gap between sittings this writer is usually working to.
    ///
    /// The middle of their own distribution rather than an average of it, so one
    /// sleepless fortnight does not move it.
    pub fn typical_gap_hours(&self) -> f64 {
        self.gaps.quantile(0.50).unwrap_or(FALLBACK_GAP_HOURS)
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
    pub fn long_gap_hours(&self) -> f64 {
        self.gaps.quantile(UPPER_QUARTILE).unwrap_or(FALLBACK_GAP_HOURS)
    }

    /// The gap this writer exceeds about one time in ten.
    ///
    /// Past this a silence has stopped being a quiet stretch and become an
    /// absence — which is the only kind worth welcoming somebody back from.
    pub fn rare_gap_hours(&self) -> f64 {
        self.gaps.quantile(RARE).unwrap_or(FALLBACK_GAP_HOURS)
    }

    /// How many gaps the distributions were actually read from.
    ///
    /// Exposed so a caller can refuse to make a claim the sample cannot support.
    /// "Longer than nine tenths of your gaps" is a real statement about sixteen
    /// of them and an opinion about three.
    pub fn measured_gaps(&self) -> usize {
        self.gaps.len
    }

    /// Posts in the most recent sitting.
    pub fn latest_sitting_posts(&self) -> usize {
        self.latest_sitting
    }

    /// When the most recent sitting began. `None` for an empty archive.
    pub fn latest_sitting_start(&self) -> Option<i64> {
        self.latest_start
    }

    /// The silence the most recent sitting ended. `None` when it is the only
    /// sitting there has ever been, which is not a silence but a beginning.
    pub fn gap_before_latest_sitting(&self) -> Option<f64> {
        self.latest_gap
    }

    /// How unusual a silence of `hours` is: the share of this writer's gaps that
    /// ran at least this long.
    pub fn gap_at_least(&self, hours: f64) -> f64 {
        self.gaps.at_least(hours)
    }

    /// How unusual a sitting of `posts` is.
    pub fn sitting_at_least(&self, posts: usize) -> f64 {
        self.sittings.at_least(posts as f64)
    }

    /// How unusual a post of `words` is.
    pub fn words_at_least(&self, words: usize) -> f64 {
        self.words.at_least(words as f64)
    }
}

/// The sittings in an archive: when each began, and how many posts it holds.
///
/// A post starts a new sitting when more than [`SESSION_GAP_MINUTES`] separates
/// it from the one before. Every gap between consecutive starts is therefore
/// larger than that threshold and comfortably positive — including for two posts
/// that share a timestamp, which are one sitting by the same rule.
fn sittings(posts: &[Morsel]) -> Vec<(i64, usize)> {
    let mut sittings: Vec<(i64, usize)> = Vec::new();
    let mut previous: Option<i64> = None;

    for post in posts {
        let at = post.created_at;

        match sittings.last_mut() {
            Some((_, count)) if previous.is_some_and(|before| minutes(at - before) <= SESSION_GAP_MINUTES) => {
                *count += 1;
            }
            _ => sittings.push((at, 1)),
        }
        previous = Some(at);
    }

    sittings
}

/// Words in a post, counted the way [`super::stats`] counts them.
fn words(post: &Morsel) -> usize {
    post.body_text.split_whitespace().count()
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

    fn gap_hours(posts: &[Morsel]) -> Vec<f64> {
        sittings(posts)
            .windows(2)
            .map(|pair| hours(pair[1].0 - pair[0].0))
            .collect()
    }

    #[test]
    fn a_handful_of_notes_in_one_go_is_a_single_sitting() {
        let one = sittings(&sitting(START, 5));
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].1, 5, "all five belong to it");

        // And the threshold is where it says it is: 45 minutes apart is still one
        // sitting, 46 is two.
        assert_eq!(sittings(&[post(START, "a"), post(START + 45 * MINUTE, "b")]).len(), 1);
        assert_eq!(sittings(&[post(START, "a"), post(START + 46 * MINUTE, "b")]).len(), 2);
    }

    #[test]
    fn gaps_are_measured_between_the_starts_of_sittings() {
        // Two sittings a day apart, each with several posts in it. The rhythm is
        // the day, not the five minutes inside either one.
        let mut posts = sitting(START, 4);
        posts.extend(sitting(START + DAY, 4));

        let gaps = gap_hours(&posts);
        assert_eq!(gaps.len(), 1);
        assert!((gaps[0] - 24.0).abs() < 1e-9, "{gaps:?}");
    }

    #[test]
    fn a_weekly_burst_writer_has_a_weekly_rhythm() {
        // The bug this module exists for. Under the mean gap between *posts* this
        // archive measured a cadence of five minutes; it is plainly a week.
        let posts: Vec<_> = (0..8)
            .flat_map(|week| sitting(START + week as i64 * 7 * DAY + 10 * HOUR, 5))
            .collect();
        let habits = Baseline::of(&posts);

        assert!((habits.typical_gap_hours() - 168.0).abs() < 1e-6, "{}", habits.typical_gap_hours());
        assert_eq!(habits.latest_sitting_posts(), 5);
    }

    #[test]
    fn an_hourly_writer_has_an_hourly_rhythm() {
        // The other end of the range, through the same formula and constants.
        let habits = Baseline::of(&run(START, 12, "a note"));
        assert!((habits.typical_gap_hours() - 1.0).abs() < 1e-9);
        assert_eq!(habits.latest_sitting_posts(), 1, "hourly posts are separate sittings");
    }

    #[test]
    fn regularity_decides_how_long_a_long_gap_is() {
        // A metronome: every sitting exactly a day apart. Nothing is forgiven
        // beyond the rhythm itself, because nothing ever varied from it.
        let metronome: Vec<_> = (0..12).flat_map(|day| sitting(START + day * DAY, 2)).collect();
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
        posts.extend((0..20).map(|day| post(switched + day * DAY, "the new slow habit")));

        let habits = Baseline::of(&posts);
        assert!((habits.typical_gap_hours() - 24.0).abs() < 1e-9, "{}", habits.typical_gap_hours());
    }

    #[test]
    fn a_single_enormous_gap_does_not_become_the_rhythm() {
        // A year off, then business as usual. A mean would still be talking about
        // the year in six months' time; the median has already forgotten it.
        let mut posts = vec![post(START, "before")];
        posts.extend(run(START + 365 * DAY, 12, "after"));

        assert!((Baseline::of(&posts).typical_gap_hours() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn nothing_to_measure_falls_back_rather_than_inventing_a_number() {
        for archive in [vec![], vec![post(START, "the very first")], sitting(START, 6)] {
            let habits = Baseline::of(&archive);
            assert_eq!(habits.typical_gap_hours(), FALLBACK_GAP_HOURS);
            assert_eq!(habits.long_gap_hours(), FALLBACK_GAP_HOURS);
        }
    }

    #[test]
    fn the_quantile_interpolates_between_order_statistics() {
        assert_eq!(Sample::of(&[1.0, 2.0, 3.0]).quantile(0.5), Some(2.0));
        assert_eq!(Sample::of(&[4.0, 1.0, 3.0, 2.0]).quantile(0.5), Some(2.5));
        assert_eq!(Sample::of(&[5.0, 1.0, 2.0, 3.0, 4.0]).quantile(0.75), Some(4.0));
        assert_eq!(Sample::of(&[7.0]).quantile(0.75), Some(7.0));
        assert_eq!(Sample::of(&[]).quantile(0.5), None);
    }

    #[test]
    fn a_tail_probability_never_reaches_zero_or_exceeds_one() {
        let sample = Sample::of(&[1.0, 2.0, 3.0, 4.0]);

        // Everything is at least the smallest value.
        assert_eq!(sample.at_least(0.0), 1.0);
        // One of four is at least 4, smoothed to two in five.
        assert!((sample.at_least(4.0) - 0.4).abs() < 1e-9);

        // The point of the smoothing: unprecedented is rare, not impossible. An
        // unsmoothed zero here is infinitely surprising and would out-shout
        // everything the pet could ever say again.
        let unprecedented = sample.at_least(1_000.0);
        assert!(unprecedented > 0.0, "an unseen value must not be impossible");
        assert!((unprecedented - 0.2).abs() < 1e-9);

        // And a sample with no history is never surprised.
        assert_eq!(Sample::of(&[]).at_least(1_000.0), 1.0);
    }

    #[test]
    fn the_rarest_thing_a_full_sample_can_report_is_one_in_seventeen() {
        // The resolution limit, stated once so the ceiling in `speech` has a
        // reason rather than a taste behind it.
        let full = Sample::of(&(0..SAMPLE).map(|i| i as f64).collect::<Vec<_>>());
        assert_eq!(full.len, SAMPLE);

        let rarest = full.at_least(f64::INFINITY);
        assert!((rarest - 1.0 / (SAMPLE as f64 + 1.0)).abs() < 1e-9, "{rarest}");
    }

    #[test]
    fn sitting_sizes_and_word_counts_are_measured_too() {
        let mut posts = sitting(START, 3);
        posts.extend(sitting(START + DAY, 7));
        let habits = Baseline::of(&posts);

        assert_eq!(habits.latest_sitting_posts(), 7);
        // Two sittings, of three and seven. Seven is the larger of the two.
        assert!(habits.sitting_at_least(7) < habits.sitting_at_least(3));

        // Every fixture post is "another note" — two words — so a longer one is
        // unlike anything in the archive.
        let long: Vec<_> = (0..40).map(|_| "word").collect();
        assert!(habits.words_at_least(long.len()) < habits.words_at_least(2));
    }
}
