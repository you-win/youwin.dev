//! What kind of writer this is, and the two places the pet's behaviour bends to
//! it.
//!
//! Every other channel in [`PetState`](super::PetState) reads the *present*: the
//! last ten posts for mood, the last fifty for form, the last sixteen gaps for
//! rhythm. A trait is the slow one underneath — a characteristic of the whole
//! archive, which changes how the pet *reacts* rather than what it measures. Two
//! people's pets should behave differently because of how they write and not
//! merely look different, and this is the module that has to earn that sentence.
//!
//! # Why two, when the design asks for three
//!
//! `familiar-design.md` names nocturnal, irregular, and laconic/prolix. Two of
//! the three had already been built by the time this module was written, by
//! machinery that landed after the traits idea was written down.
//!
//! **Nocturnal** — "the pet stops sleeping at 3am and inverts its phase offsets"
//! — is what [`energy::profile`] already does. Phases are cut from the archive's
//! own histogram, so a writer who only ever posts at three in the morning has
//! 03:00 as their `Peak` within a fortnight, and `sleep` only ever fires in
//! `Deep`; `the_learned_rhythm_displaces_the_assumed_one` over in that module is
//! the assertion. There is also no honest way to *detect* it. The site is UTC
//! and deliberately does not guess at zones, so nocturnal measured against the
//! clock calls a writer eleven zones east a night owl for posting after
//! breakfast; and nocturnal measured against the learned profile is
//! self-defeating, because the more of an archive that lands at night the less
//! the night reads as unusual.
//!
//! **Irregular** — "it forgives longer" — is the decay half-life, which is twice
//! the writer's own 75th-percentile gap. `regularity_decides_how_long_a_long_gap_is`
//! in [`baseline`](super::baseline) is literally a test that an irregular writer
//! is forgiven longer than a metronome with the same median. A trait multiplying
//! that again would be counting one piece of evidence twice, which is the
//! failure the whole "tails, not scores" discipline in [`speech`](super::speech)
//! exists to avoid.
//!
//! What was left is the two places the pet is still the same creature for
//! everybody.
//!
//! **Energy counts posts and never words.** [`energy::step`] is worth a fixed
//! amount per post, so a weekly two-thousand-word essay moves the pet exactly as
//! far as "brb" does. That is the same shape as the bug [`baseline`] was built
//! for — an archive measured in a unit that is wrong for a whole class of writer
//! — and it leaves the essayist with a pet permanently on the floor no matter
//! how much they write.
//!
//! **The phase cut has no measure of its own confidence.** `energy::phases`
//! always names a densest four-hour block, including for a histogram that is
//! flat. A writer who genuinely writes at every hour still gets ±0.10 and ±0.15
//! swings, and a pet that sleeps, off a peak that is an artefact of a cut having
//! to pick something.
//!
//! # Neutral is whatever the pet already did
//!
//! Both traits are ratios against a reference picked so an ordinary archive
//! lands on exactly 1.0 and the arithmetic downstream is the arithmetic that was
//! there before. A trait can bend the pet for a writer the machine currently
//! mis-serves; it cannot move one that was already right, and no archive changes
//! behaviour on the day this module is deployed unless it was being read wrongly
//! the day before.
//!
//! That is also why neither is a flag. The effect is continuous, so nothing
//! lurches as an archive drifts across a line, and the names on `/familiar` are
//! a rounding of the number for the page rather than the thing being applied.

use crate::familiar::{
    Morsel, baseline,
    energy::{assumed_peak_share, peak_share},
};

/// Words in an ordinary note, and so the length at which this trait does
/// nothing at all.
///
/// An absolute count, unlike every other reference in the familiar, and it is
/// worth being clear about why that is allowed here when it was not allowed for
/// nocturnal. An hour needs a timezone before it means anything and the site
/// does not have one. A word does not: thirty of them is a couple of sentences
/// for everybody, which is what a note on a microblog is.
const NOTE_WORDS: f64 = 30.0;

/// Bounds on what writing at length is worth.
///
/// Being wordy is worth at most twice an ordinary note and being terse at least
/// half of one, which is the same span [`energy`] already allows the cadence
/// factor. Effort is sublinear in length — four times the words is not four
/// times the sitting — so the ratio is taken through a square root and these
/// bind at a median of roughly ten words and roughly a hundred and twenty.
const MIN_LENGTH: f64 = 0.5;
const MAX_LENGTH: f64 = 2.0;

/// Posts before there is any character to read.
///
/// One [`baseline::SAMPLE`], for the same reason that constant exists: below it
/// a median is being taken over a handful of numbers, and an archive whose first
/// three posts happened to be long is not the work of an essayist. Under this
/// the traits are neutral, which is exactly what the pet did before they existed.
const ENOUGH_POSTS: usize = baseline::SAMPLE;

/// The share of any archive that lands in any four hours by arithmetic alone.
///
/// The zero of the focus scale. A perfectly flat histogram still has a densest
/// four-hour block — `energy::phases` has to name one — and it holds a sixth of
/// the posts for no reason other than that a sixth of the day is four hours.
const CHANCE_SHARE: f64 = 4.0 / 24.0;

/// Focus below which the pet stops treating its own deep hours as night.
///
/// Half the concentration of the schedule the pet assumes about a writer nobody
/// has watched. Below it the peak block is closer to chance than to a habit, so
/// the hours on the other side of it are not the writer being asleep — they are
/// the remainder after a cut that had to fall somewhere.
///
/// The one threshold in this module, and it is shared: the hour at which the pet
/// stops drawing `zZ` is the same one at which `/familiar` starts calling the
/// archive scattered, so what the pet does and what the page says about it
/// cannot come apart.
const KEEPS_HOURS: f64 = 0.5;

/// Lengths outside which the archive is worth naming on the page.
///
/// A median of about nineteen words and about forty-seven. Wider than the point
/// at which the trait starts having an effect, because the effect is continuous
/// and a *name* should not appear for an archive that is barely off ordinary.
const TERSE: f64 = 0.8;
const PROLIX: f64 = 1.25;

/// Slow-moving characteristics of the whole archive.
///
/// Two numbers rather than two enums: both are applied continuously, and the
/// labels are derived from them for display only. `Copy` so [`PetState`] can
/// stay `Copy` — the whole state is still five enums and a handful of numbers.
///
/// [`PetState`]: super::PetState
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Traits {
    length: f64,
    focus: f64,
}

/// Neutral: an archive with nothing yet to say about itself behaves exactly the
/// way every archive did before this module existed.
impl Default for Traits {
    fn default() -> Self {
        Self {
            length: 1.0,
            focus: 1.0,
        }
    }
}

impl Traits {
    /// Reads the character out of an archive.
    ///
    /// `profile` is the circadian curve [`super::compute`] has already built,
    /// handed in rather than rebuilt. That is not only to save a pass: focus is
    /// a measure of confidence in *the cut the phase was taken from*, so reading
    /// it off a second curve would let the two disagree about a writer's hours
    /// while claiming to describe the same ones.
    pub fn of(posts: &[Morsel], profile: &[f64; 24]) -> Self {
        if posts.len() < ENOUGH_POSTS {
            return Self::default();
        }

        Self {
            length: length(posts),
            focus: focus(profile),
        }
    }

    /// What one post from this writer is worth, against one note.
    ///
    /// Multiplied into [`energy`]'s burst, so the pet answers how much was
    /// written rather than how many times the button was pressed. Deliberately a
    /// property of the *archive* and not of the post in hand: per-post it would
    /// make one pasted quotation a spike, and the pet's whole design is that
    /// slow things move slowly.
    pub fn length(self) -> f64 {
        self.length
    }

    /// How much of the circadian offset survives.
    ///
    /// One for a writer whose hours are at least as concentrated as the schedule
    /// assumed about a stranger, nought for one who writes at every hour equally.
    /// It can only ever damp: a sharper habit than the assumed bump is already
    /// getting the full offset and there is nothing above full to give it.
    pub fn focus(self) -> f64 {
        self.focus
    }

    /// Whether this archive's deep hours are the writer being asleep or an
    /// artefact of where the peak block fell.
    pub fn keeps_hours(self) -> bool {
        self.focus >= KEEPS_HOURS
    }

    /// The characteristics worth naming, for the page.
    ///
    /// Departures only. An archive of note-sized posts written to at consistent
    /// hours is not "ordinary, punctual" — it goes unremarked on, which is the
    /// same rule [`speech`](super::speech) follows and for the same reason: a
    /// label that is always there is read once and then never again. It also
    /// keeps the page clear of anything that could be read as a score, since
    /// there is no pole here that is the good one.
    pub fn labels(self) -> Vec<&'static str> {
        let length = if self.length <= TERSE {
            Some("terse")
        } else if self.length >= PROLIX {
            Some("prolix")
        } else {
            None
        };

        let hours = (!self.keeps_hours()).then_some("scattered");

        [length, hours].into_iter().flatten().collect()
    }
}

/// The archive's typical post, as a multiple of a note.
fn length(posts: &[Morsel]) -> f64 {
    let mut words: Vec<f64> = posts
        .iter()
        .map(|post| post.body_text.split_whitespace().count() as f64)
        .collect();

    let Some(typical) = median(&mut words) else {
        return 1.0;
    };

    (typical / NOTE_WORDS).sqrt().clamp(MIN_LENGTH, MAX_LENGTH)
}

/// How much the peak block means, against how much it would mean for somebody
/// who keeps hours.
///
/// The reference is the assumed schedule's own concentration, which makes the
/// scale honest at both ends by construction: an archive too young to have
/// displaced the prior *is* the prior, so it reads as exactly one and its pet
/// behaves as it always did, and the number then moves continuously as the
/// histogram takes over.
fn focus(profile: &[f64; 24]) -> f64 {
    let kept = assumed_peak_share() - CHANCE_SHARE;

    // Structurally positive — the assumed schedule is a bump, not a flat line —
    // but a division that could produce a NaN sits directly upstream of the
    // energy a page is rendered from, and a pet nobody can draw is a worse
    // outcome than a pet with no opinion about hours.
    if kept <= 0.0 {
        return 1.0;
    }

    ((peak_share(profile) - CHANCE_SHARE) / kept).clamp(0.0, 1.0)
}

/// The middle of `values`, interpolating between the two order statistics it
/// falls between. `None` for an empty slice.
///
/// The convention [`baseline`]'s quantiles use, so "typical" means the same
/// thing on both sides of the module boundary. Sorts in place, over the whole
/// archive rather than a recent window — that window is precisely what makes
/// baseline a reading of the present and this one a reading of the character.
fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);

    let position = 0.5 * (values.len() - 1) as f64;
    let below = position.floor() as usize;
    let above = position.ceil() as usize;

    Some(values[below] + (values[above] - values[below]) * (position - below as f64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::familiar::{
        energy,
        fixture::{DAY, HOUR, START, post},
    };

    /// An archive of `count` posts, one a day, each `words` long.
    fn archive(count: i64, words: usize) -> Vec<Morsel> {
        let body = vec!["word"; words].join(" ");
        (0..count)
            .map(|day| post(START + day * DAY, &body))
            .collect()
    }

    fn read(posts: &[Morsel], now: i64) -> Traits {
        Traits::of(posts, &energy::profile(posts, now))
    }

    #[test]
    fn an_ordinary_archive_behaves_exactly_as_it_did_before() {
        // The rule the whole module is calibrated to: note-sized posts at
        // consistent hours change nothing and are named as nothing.
        let posts = archive(30, NOTE_WORDS as usize);
        let character = read(&posts, START + 30 * DAY);

        assert!((character.length() - 1.0).abs() < 1e-9, "{}", character.length());
        assert_eq!(character.focus(), 1.0);
        assert!(character.keeps_hours());
        assert!(character.labels().is_empty());
    }

    #[test]
    fn a_young_archive_has_no_character_to_read() {
        // Fifteen one-word posts is a terse archive by arithmetic and not by
        // habit. Under the sample the traits are neutral, which is the behaviour
        // that was there before they existed.
        let posts = archive(ENOUGH_POSTS as i64 - 1, 1);
        assert_eq!(read(&posts, START + 20 * DAY), Traits::default());

        // One more post and it is entitled to an opinion about itself.
        let posts = archive(ENOUGH_POSTS as i64, 1);
        assert_eq!(read(&posts, START + 20 * DAY).length(), MIN_LENGTH);
    }

    #[test]
    fn an_essayist_and_a_fragment_writer_are_told_apart() {
        let essays = read(&archive(20, 400), START + 20 * DAY);
        let notes = read(&archive(20, 25), START + 20 * DAY);
        let fragments = read(&archive(20, 4), START + 20 * DAY);

        assert_eq!(essays.length(), MAX_LENGTH);
        assert_eq!(fragments.length(), MIN_LENGTH);
        assert!(
            notes.length() > fragments.length() && notes.length() < essays.length(),
            "a note-writer sits between them: {}",
            notes.length(),
        );

        assert_eq!(essays.labels(), ["prolix"]);
        assert_eq!(fragments.labels(), ["terse"]);
        assert!(notes.labels().is_empty(), "twenty-five words is a note");
    }

    #[test]
    fn one_long_post_does_not_make_an_essayist() {
        // The reason this is a median over the archive rather than a mean, and
        // the reason it is not read off the post in hand: a pasted quotation is
        // not a change of character.
        let mut posts = archive(30, 5);
        posts.push(post(START + 30 * DAY, &vec!["word"; 4_000].join(" ")));

        assert_eq!(read(&posts, START + 30 * DAY).length(), MIN_LENGTH);
    }

    #[test]
    fn the_assumed_schedule_is_exactly_what_keeping_hours_means() {
        // The calibration that makes focus safe to deploy: an archive too young
        // to have displaced the prior *is* the prior, so it reads as exactly one
        // and its pet keeps every bit of the offset it had yesterday. Exact
        // rather than approximate because it is exact by construction — the same
        // curve, measured by the same search.
        let posts = archive(20, 10);
        let day_one = Traits::of(&posts, &energy::profile(&posts[..1], START));

        assert_eq!(day_one.focus(), 1.0);
        assert!(day_one.keeps_hours());

        // And it comes off the ceiling only as the histogram earns it. A single
        // post an hour in has displaced a three-hundredth of the guess, so the
        // number moves by about that much and not by a step.
        let hour_in = Traits::of(&posts, &energy::profile(&posts[..1], START + HOUR));
        assert!(
            (1.0 - hour_in.focus()) < 0.01,
            "one hour of evidence moved focus to {}",
            hour_in.focus(),
        );
    }

    #[test]
    fn a_writer_at_every_hour_has_no_hours() {
        // Twenty-four posts, one in each hour of the day, spread over enough
        // days that the histogram has fully displaced the assumed schedule. The
        // peak block `energy::phases` names here holds exactly the sixth of the
        // archive that four hours holds by arithmetic, and means nothing.
        let flat: Vec<_> = (0..24)
            .map(|i| post(START + i * DAY + i * HOUR, "a note"))
            .collect();
        let character = read(&flat, START + 24 * DAY);

        assert_eq!(character.focus(), 0.0);
        assert!(!character.keeps_hours(), "there is no night here to sleep through");
        assert!(character.labels().contains(&"scattered"));
    }

    #[test]
    fn a_sharp_habit_is_never_amplified() {
        // The nocturnal case, and the point at which it stops being a trait: a
        // writer who only ever posts at three in the morning is not scattered,
        // and there is nothing above the full offset to hand them. The phase
        // machinery has already moved their peak to 03:00 by itself.
        let nights: Vec<_> = (0..24)
            .map(|day| post(START + day * DAY + 3 * HOUR, "another late one"))
            .collect();
        let character = read(&nights, START + 24 * DAY);

        assert_eq!(character.focus(), 1.0, "clamped, not amplified");
        assert!(character.keeps_hours());
        assert!(!character.labels().contains(&"scattered"));
    }

    #[test]
    fn focus_falls_off_continuously_rather_than_switching() {
        // Nothing may lurch as an archive drifts. Walk a habit from sharp to
        // flat by moving a growing share of the posts off the hour, and the
        // number has to come down without stepping.
        let mut previous = f64::INFINITY;

        for scattered in 0..=24 {
            let posts: Vec<_> = (0..48)
                .map(|i| {
                    let hour = if i % 48 < scattered * 2 { i % 24 } else { 3 };
                    post(START + i * DAY + hour * HOUR, "a note")
                })
                .collect();

            let focus = read(&posts, START + 48 * DAY).focus();
            assert!(focus <= previous + 1e-9, "{focus} rose above {previous}");
            previous = focus;
        }

        assert!(previous < KEEPS_HOURS, "the far end is scattered: {previous}");
    }

    #[test]
    fn the_middle_of_an_even_sample_sits_between_its_neighbours() {
        assert_eq!(median(&mut []), None);
        assert_eq!(median(&mut [7.0]), Some(7.0));
        assert_eq!(median(&mut [3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&mut [4.0, 1.0, 3.0, 2.0]), Some(2.5));
    }

    #[test]
    fn an_empty_archive_reads_as_neutral_and_does_not_panic() {
        assert_eq!(read(&[], START), Traits::default());
    }
}
