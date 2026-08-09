//! The numbers under the picture.
//!
//! The kaomoji is the statistic — this is the part that admits what it is made
//! of. Everything here is derived from the same posts the state machine reads,
//! so the sheet can never disagree with the face above it.

use crate::familiar::{Blend, Morsel, PetState, Topic, baseline::Baseline};

const DAY_MILLIS: i64 = 86_400_000;

/// Posts per day the character sheet treats as a full score for `STR`.
const PROLIFIC_POSTS_PER_DAY: f64 = 2.0;

/// Posts at which `WIS` tops out. Not a difficulty curve — an archive of a
/// hundred posts has genuinely seen more than one of five.
const SEASONED_POSTS: f64 = 100.0;

/// What the archive is, in numbers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vitals {
    pub posts: usize,
    /// Calendar days from the first post to now, inclusive of both — a blog
    /// written in one afternoon is a day old, not zero days old, and `age_days`
    /// divides.
    ///
    /// Counted in whole UTC days rather than as elapsed time floored to days, so
    /// it is measured the same way [`Vitals::streak_days`] is. Flooring the
    /// interval instead lets an archive report a 22-day streak while claiming to
    /// be 21 days old, which is two true statements that cannot both be read on
    /// the same line.
    pub age_days: i64,
    pub words: usize,
    pub words_per_post: usize,
    /// Consecutive days ending at the most recent day anything was written.
    pub streak_days: i64,
    /// Whether that streak is still going: something was written today or
    /// yesterday. A finished streak is a fact about the past and does not burn.
    pub streak_alive: bool,
    /// The typical gap between *sittings*, not between posts — how often this
    /// writer sits down, which is the thing a reader means by cadence and the
    /// thing the pet's patience is measured in. See
    /// [`Baseline::typical_gap_hours`].
    pub cadence_hours: f64,
}

/// Five derived scores, each a percentage.
///
/// Explicitly a character sheet rather than a dashboard: these are readings of
/// one writer's habits, and dressing them as engagement metrics would make them
/// mean something they do not. Nothing here is a goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sheet {
    /// Current energy.
    pub vitality: u8,
    /// Posting density against a prolific baseline.
    pub strength: u8,
    /// How spread out the diet is — Shannon entropy over the six topics.
    pub curiosity: u8,
    /// Accumulated archive.
    pub wisdom: u8,
    /// Share of the diet that is abstract.
    pub magic: u8,
}

impl Sheet {
    /// In display order, with the labels the sheet shows.
    pub fn rows(self) -> [(&'static str, u8); 5] {
        [
            ("VIT", self.vitality),
            ("STR", self.strength),
            ("CUR", self.curiosity),
            ("WIS", self.wisdom),
            ("MAG", self.magic),
        ]
    }
}

/// Measures the archive at `now`. `posts` must be sorted oldest first.
pub fn vitals(posts: &[Morsel], now: i64) -> Vitals {
    let words = posts
        .iter()
        .map(|post| post.body_text.split_whitespace().count())
        .sum();

    let age_days = posts.first().map_or(1, |first| {
        (day_of(now) - day_of(first.created_at)).max(0) + 1
    });

    let (streak_days, streak_alive) = streak(posts, now);

    Vitals {
        posts: posts.len(),
        age_days,
        words,
        words_per_post: words / posts.len().max(1),
        streak_days,
        streak_alive,
        cadence_hours: Baseline::of(posts).typical_gap_hours(),
    }
}

/// Rolls the character sheet.
pub fn sheet(state: &PetState, vitals: &Vitals, diet: Blend) -> Sheet {
    let expected = vitals.age_days as f64 * PROLIFIC_POSTS_PER_DAY;
    // `entropy` is in bits over six topics, so `log2(6)` is the ceiling — a diet
    // divided evenly across all of them.
    let breadth = diet.entropy() / (Topic::ALL.len() as f64).log2();

    Sheet {
        vitality: percent(state.energy),
        strength: percent(vitals.posts as f64 / expected.max(1.0)),
        curiosity: percent(breadth),
        wisdom: percent(vitals.posts as f64 / SEASONED_POSTS),
        magic: percent(diet.weight(Topic::Abstract)),
    }
}

/// A `0.0..=1.0` ratio as a percentage, clamped. NaN reads as zero rather than
/// panicking on the cast — no visitor should get a 500 because of a division.
pub fn percent(ratio: f64) -> u8 {
    if !ratio.is_finite() {
        return 0;
    }
    (ratio * 100.0).round().clamp(0.0, 100.0) as u8
}

/// Consecutive days written on, counting back from the last one, and whether it
/// is still running.
///
/// Days are UTC, like every other timestamp on the site. That means a streak
/// turns over at midnight UTC rather than at the writer's midnight — the same
/// trade `view::time_fmt` makes, and for the same reason: guessing a zone would
/// make the page vary by reader and take the whole surface out of the edge cache.
fn streak(posts: &[Morsel], now: i64) -> (i64, bool) {
    let mut days: Vec<i64> = posts.iter().map(|post| day_of(post.created_at)).collect();
    days.dedup();

    let Some(last) = days.last().copied() else {
        return (0, false);
    };

    let mut streak = 1;
    for pair in days.windows(2).rev() {
        if pair[1] - pair[0] == 1 {
            streak += 1;
        } else {
            break;
        }
    }

    // Yesterday still counts: it is only this evening's post that is missing,
    // and a streak that died at midnight would be wrong for most of the day.
    (streak, day_of(now) - last <= 1)
}

/// Whole days since the epoch, UTC. Exact for the same reason
/// [`super::energy`]'s hour arithmetic is: unix time has no leap seconds and its
/// epoch is midnight UTC.
fn day_of(millis: i64) -> i64 {
    millis.div_euclid(DAY_MILLIS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::familiar::{
        compute,
        fixture::{DAY, HOUR, START, post, run},
        topics,
    };

    #[test]
    fn an_empty_archive_measures_as_empty_without_dividing_by_zero() {
        let measured = vitals(&[], START);
        assert_eq!(measured.posts, 0);
        assert_eq!(measured.words, 0);
        assert_eq!(measured.words_per_post, 0);
        assert_eq!(measured.age_days, 1);
        assert_eq!(measured.streak_days, 0);
        assert!(!measured.streak_alive);

        let state = compute(&[], None, START);
        let rolled = sheet(&state, &measured, Blend::default());
        assert_eq!(rolled.strength, 0);
        assert_eq!(rolled.curiosity, 0);
        assert_eq!(rolled.wisdom, 0);
        assert_eq!(rolled.magic, 0);
    }

    #[test]
    fn words_are_counted_and_averaged() {
        let posts = [
            post(START, "one two three"),
            post(START + HOUR, "four five"),
        ];
        let measured = vitals(&posts, START + 2 * HOUR);
        assert_eq!(measured.words, 5);
        assert_eq!(measured.words_per_post, 2, "integer division, deliberately");
    }

    #[test]
    fn age_and_streak_are_measured_in_the_same_days() {
        // Both count calendar days, so a blog written on every one of its days
        // cannot report a streak longer than its own age.
        let posts: Vec<_> = (0..22)
            .map(|day| post(START + day * DAY + 20 * HOUR, "a note"))
            .collect();

        // Late on the last day: 21 whole days have elapsed, but 22 have been
        // written on.
        let measured = vitals(&posts, START + 21 * DAY + 23 * HOUR);
        assert_eq!(measured.streak_days, 22);
        assert_eq!(measured.age_days, 22);

        // A blog written entirely this afternoon is one day old, not zero.
        let today = vitals(&[post(START + 14 * HOUR, "a note")], START + 15 * HOUR);
        assert_eq!(today.age_days, 1);
    }

    #[test]
    fn a_streak_counts_back_from_the_last_day_written_on() {
        // Days 0, 1, 2, then a gap, then days 5, 6.
        let posts: Vec<_> = [0, 1, 2, 5, 6]
            .into_iter()
            .map(|day| post(START + day * DAY + 9 * HOUR, "a note"))
            .collect();

        let measured = vitals(&posts, START + 6 * DAY + 12 * HOUR);
        assert_eq!(measured.streak_days, 2, "the run of 3 was broken");
        assert!(measured.streak_alive, "written today");
    }

    #[test]
    fn several_posts_in_one_day_are_one_day_of_streak() {
        let posts = run(START, 6, "a note");
        let measured = vitals(&posts, START + 6 * HOUR);
        assert_eq!(measured.streak_days, 1);
        assert!(measured.streak_alive);
    }

    #[test]
    fn a_streak_survives_one_quiet_day_and_no_more() {
        let posts: Vec<_> = [0, 1, 2]
            .into_iter()
            .map(|day| post(START + day * DAY, "a note"))
            .collect();

        let yesterday = vitals(&posts, START + 3 * DAY + HOUR);
        assert_eq!(yesterday.streak_days, 3);
        assert!(yesterday.streak_alive, "yesterday still counts");

        let stale = vitals(&posts, START + 4 * DAY + HOUR);
        assert_eq!(stale.streak_days, 3, "the streak happened, it is just over");
        assert!(!stale.streak_alive);
    }

    #[test]
    fn curiosity_spans_a_single_topic_to_all_six() {
        let state = compute(&[], None, START);
        let measured = vitals(&[], START);

        let narrow = Blend::from_weights([1.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(sheet(&state, &measured, narrow).curiosity, 0);

        let wide = Blend::from_weights([1.0; 6]);
        assert_eq!(sheet(&state, &measured, wide).curiosity, 100);
    }

    #[test]
    fn scores_are_clamped_rather_than_wrapping() {
        // Someone who posts fifty times a day for a day, about nothing but
        // philosophy. Every score that could overflow is pinned instead.
        let posts = run(START, 50, "a thought about time, void, recursion, paradox");
        let now = START + 50 * HOUR;

        let state = compute(&posts, None, now);
        let measured = vitals(&posts, now);
        let rolled = sheet(&state, &measured, topics::classify(&posts));

        assert_eq!(rolled.strength, 100);
        assert_eq!(rolled.magic, 100);
        assert!(rolled.wisdom <= 100);
        assert!(rolled.vitality <= 100);

        assert_eq!(percent(f64::NAN), 0);
        assert_eq!(percent(-1.0), 0);
        assert_eq!(percent(17.0), 100);
    }

    #[test]
    fn the_sheet_reads_out_in_a_fixed_order() {
        let labels: Vec<_> = Sheet {
            vitality: 1,
            strength: 2,
            curiosity: 3,
            wisdom: 4,
            magic: 5,
        }
        .rows()
        .into_iter()
        .map(|(label, _)| label)
        .collect();

        assert_eq!(labels, ["VIT", "STR", "CUR", "WIS", "MAG"]);
    }
}
