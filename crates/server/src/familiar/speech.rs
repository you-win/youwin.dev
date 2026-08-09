//! One line, and the rule that picks it: **the pet says the most surprising true
//! thing about the archive.**
//!
//! A shuffled table of stock phrases would be dynamic for a week and wallpaper
//! after that, because the interesting sentence is never the one that is always
//! available. "47 posts" is true every day and worth saying on none of them; "it
//! has not been fed in nine days" is true rarely and is the only thing worth
//! saying when it is. So candidates are not weighted by taste. Each one reports
//! the probability that the archive would look *at least this extreme* under the
//! writer's own history, the line with the least likely observation wins, and a
//! floor means that on an ordinary day the pet says nothing at all.
//!
//! That last part is the load-bearing one. A pet that always has a line will
//! spend most of its life saying something dull, and one dull line teaches you
//! to stop reading the interesting ones.
//!
//! **Creature voice, everywhere.** Nothing here addresses a reader. The public
//! widget is read by strangers, so a line in the second person would be telling
//! *them* about someone else's habits, and the same sentence has to work on both
//! surfaces or there are two tables to keep true instead of one. The pet is "it",
//! the archive is what it eats, and the writer is never spoken to. A test pins
//! this, because it is the kind of rule that erodes one convenient phrasing at a
//! time.
//!
//! Comparability across candidates is the whole trick, and it comes from every
//! probability meaning the same thing — a tail, not a score. What it costs is a
//! ceiling: [`Sample::at_least`](super::baseline) is smoothed against a sixteen-long
//! sample and cannot tell one-in-twenty from one-in-ever, so surprise is capped
//! at [`MAX_BITS`]. The useful side effect is that no single dimension can
//! monopolise the line — once two candidates are both pinned at the ceiling they
//! take turns, by the day.

use crate::familiar::{Baseline, Mood, Morsel, PetState, baseline, energy};

/// Surprise below which the pet says nothing.
///
/// Two bits is one-in-four: something that happens this often is not news, and a
/// line that reports it is the wallpaper this module exists to avoid.
const MIN_BITS: f64 = 2.0;

/// Surprise above which nothing is more surprising.
///
/// The sample behind these tails is sixteen long, so the rarest thing it can
/// honestly report is a shade under one in seventeen — about 4.1 bits. Claiming
/// more would be reading precision out of a sample that does not have it.
const MAX_BITS: f64 = 4.0;

/// How close to the leader another candidate has to be to share the day with it.
const NEAR_BITS: f64 = 0.5;

/// Words below which a "long one" is not long, whatever the archive's habits.
///
/// Someone who writes in five-word fragments has a distribution in which eleven
/// words is genuinely unprecedented, and "it has swallowed something long" about
/// a sentence is a joke the pet is not in on.
const LONG_ENOUGH_WORDS: usize = 40;

const HOUR_MILLIS: i64 = 3_600_000;
const DAY_MILLIS: i64 = 24 * HOUR_MILLIS;

/// What the pet has to say, and how surprised it is.
#[derive(Debug, Clone, PartialEq)]
pub struct Utterance {
    pub line: String,
    /// Bits of surprise behind it. Not rendered — kept because "why did it say
    /// that" is otherwise unanswerable, and the tests assert on it.
    pub bits: f64,
}

/// One thing that could be said, and how unlikely the observation behind it is.
struct Candidate {
    line: String,
    /// P(the archive looks at least this extreme), under the writer's own
    /// history. A tail, never a score — that is what makes two candidates
    /// comparable at all.
    probability: f64,
}

/// The most surprising true thing, or nothing.
///
/// `posts` must be sorted oldest first. `moods` is the whole-archive
/// distribution, commonest first, as [`super::mood::distribution`] returns it.
pub fn speak(
    posts: &[Morsel],
    habits: &Baseline,
    state: &PetState,
    moods: &[(Mood, f64)],
    now: i64,
) -> Option<Utterance> {
    let candidates = [
        silence(posts, habits, now),
        abundance(posts, habits, now),
        length(posts, habits, now),
        odd_hour(posts, now),
        rare_mood(state, moods, posts, now),
    ];

    let mut ranked: Vec<(f64, String)> = candidates
        .into_iter()
        .flatten()
        .map(|candidate| (bits(candidate.probability), candidate.line))
        .filter(|(bits, _)| *bits >= MIN_BITS)
        .collect();

    ranked.sort_by(|(a, _), (b, _)| b.total_cmp(a));

    // Everything within half a bit of the leader is, as far as a sample this size
    // can tell, equally worth saying. Rotating between them by the day is what
    // stops a pet parked at the ceiling from repeating itself all week, and the
    // day is the unit because the five-minute snapshot has to agree with itself
    // across every page that draws from it.
    let leader = ranked.first()?.0;
    let sharing = ranked
        .iter()
        .take_while(|(bits, _)| leader - bits <= NEAR_BITS)
        .count();
    let (bits, line) = ranked.swap_remove(rotate(now.div_euclid(DAY_MILLIS), sharing));

    Some(Utterance { line, bits })
}

/// Surprise in bits, capped. A probability of zero cannot arrive — every tail
/// here is smoothed — but the clamp says so rather than trusting it.
fn bits(probability: f64) -> f64 {
    if probability <= 0.0 {
        return MAX_BITS;
    }
    (-probability.log2()).clamp(0.0, MAX_BITS)
}

/// Which of `count` equally-good candidates today gets the line.
///
/// A hash rather than `day % count`, so the choice does not march predictably
/// through the list as the days do, and so adding a candidate does not shift
/// every previous day's answer by one.
fn rotate(day: i64, count: usize) -> usize {
    let mut mixed = (day as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    mixed ^= mixed >> 33;
    mixed = mixed.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    mixed ^= mixed >> 33;

    (mixed % count.max(1) as u64) as usize
}

/// Whether the latest post is recent enough for anything about it to be news.
///
/// Everything except [`silence`] describes the last thing written, and "it has
/// swallowed something long" is a strange thing to hear about a post from three
/// weeks ago. One typical gap is the writer's own definition of "just now".
fn is_current(posts: &[Morsel], habits: &Baseline, now: i64) -> bool {
    posts.last().is_some_and(|last| {
        hours(now - last.created_at) <= habits.typical_gap_hours()
    })
}

/// How long it has been, against how long it usually is.
fn silence(posts: &[Morsel], habits: &Baseline, now: i64) -> Option<Candidate> {
    let last = posts.last()?;
    let quiet = hours(now - last.created_at);
    if quiet <= 0.0 {
        return None;
    }

    Some(Candidate {
        line: format!("it has not been fed in {}.", duration(quiet)),
        probability: habits.gap_at_least(quiet),
    })
}

/// How much landed in this sitting, against how much usually does.
fn abundance(posts: &[Morsel], habits: &Baseline, now: i64) -> Option<Candidate> {
    if !is_current(posts, habits, now) {
        return None;
    }

    let sitting = habits.latest_sitting_posts();
    if sitting < 2 {
        return None;
    }

    Some(Candidate {
        line: format!("{sitting} in a single sitting."),
        probability: habits.sitting_at_least(sitting),
    })
}

/// How long the last post was, against how long they usually are.
fn length(posts: &[Morsel], habits: &Baseline, now: i64) -> Option<Candidate> {
    if !is_current(posts, habits, now) {
        return None;
    }

    let words = posts.last()?.body_text.split_whitespace().count();
    if words < LONG_ENOUGH_WORDS {
        return None;
    }

    Some(Candidate {
        line: "it has swallowed something long.".to_owned(),
        probability: habits.words_at_least(words),
    })
}

/// Whether the last post landed in an hour this archive keeps.
///
/// The one candidate whose probability comes from a **prior** rather than from
/// observations, and so the only one that needs a gate. Every other tail here is
/// smoothed against a sample that is simply empty early on, which returns a
/// probability of one and falls under the floor by itself. This one is judged
/// against the circadian profile, which is a *blend* of what the archive has
/// shown and an assumed human schedule — so on day one it will happily announce
/// that a midnight post came at an odd hour, having never seen this writer at
/// any hour at all. It was doing exactly that before this gate existed.
///
/// Two conditions, both borrowed rather than invented: a fortnight, which is
/// when [`energy::COLD_START_DAYS`] says the guess has finished being displaced
/// by the evidence, and a full [`baseline::SAMPLE`] of posts, because a
/// histogram of three spikes makes the other twenty-one hours "unusual" by
/// default.
fn odd_hour(posts: &[Morsel], now: i64) -> Option<Candidate> {
    let last = posts.last()?;
    if now - last.created_at > DAY_MILLIS || posts.len() < baseline::SAMPLE {
        return None;
    }

    let watched_days = hours(now - posts.first()?.created_at) / 24.0;
    if watched_days < energy::COLD_START_DAYS {
        return None;
    }

    let profile = energy::profile(posts, now);
    let density = profile[energy::hour_of(last.created_at)];

    // The tail: the chance a post lands in an hour at least this unlikely.
    let probability = profile.iter().filter(|hour| **hour <= density).sum();

    Some(Candidate {
        line: "fed at an hour it does not keep.".to_owned(),
        probability,
    })
}

/// How often this archive has felt the way it currently does.
fn rare_mood(
    state: &PetState,
    moods: &[(Mood, f64)],
    posts: &[Morsel],
    now: i64,
) -> Option<Candidate> {
    // Neutral is the absence of a feeling, not a rare one, and a mood worn from
    // weeks ago is not news.
    if state.mood == Mood::Neutral || now - posts.last()?.created_at > DAY_MILLIS {
        return None;
    }

    let share = moods
        .iter()
        .find(|(mood, _)| *mood == state.mood)
        .map(|(_, share)| *share)?;

    Some(Candidate {
        line: format!("it has seldom felt this {}.", state.mood.as_str()),
        probability: moods
            .iter()
            .filter(|(_, other)| *other <= share)
            .map(|(_, other)| other)
            .sum(),
    })
}

/// A span, in the largest unit that does not make it a decimal.
fn duration(hours: f64) -> String {
    let days = hours / 24.0;

    if hours < 2.0 {
        "an hour".to_owned()
    } else if hours < 48.0 {
        format!("{hours:.0} hours")
    } else if days < 14.0 {
        format!("{days:.0} days")
    } else {
        format!("{:.0} weeks", days / 7.0)
    }
}

fn hours(millis: i64) -> f64 {
    millis as f64 / HOUR_MILLIS as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::familiar::{
        compute,
        fixture::{DAY, HOUR, START, post, post_feeling, run},
        mood,
    };

    /// Everything `speak` needs, from one archive at one moment.
    fn said(posts: &[Morsel], now: i64) -> Option<Utterance> {
        let habits = Baseline::of(posts);
        let state = compute(posts, None, now);
        speak(posts, &habits, &state, &mood::distribution(posts), now)
    }

    #[test]
    fn an_ordinary_day_is_not_worth_remarking_on() {
        // A metronome, read one typical gap after its last post: nothing about
        // this is unusual, so the pet keeps quiet rather than reaching for filler.
        let posts: Vec<_> = (0..20).map(|day| post(START + day * DAY, "a note")).collect();
        assert_eq!(said(&posts, START + 19 * DAY + 12 * HOUR), None);
    }

    #[test]
    fn an_empty_archive_says_nothing_and_does_not_panic() {
        assert_eq!(said(&[], START), None);
        // One post, and no history at all to judge it against.
        assert_eq!(said(&[post(START, "the first")], START + HOUR), None);
    }

    #[test]
    fn nothing_is_claimed_about_hours_before_they_have_been_learned() {
        // The failure this gate exists for: with a single midnight post the pet
        // announced "fed at an hour it does not keep", which it could only have
        // got from the *assumed* schedule — a guess about a writer it had never
        // watched. A young archive has no opinion about anybody's hours.
        let fortnight_short: Vec<_> = (0..20)
            .map(|i| post(START + i * HOUR, "a note"))
            .collect();
        let spoken = said(&fortnight_short, START + 20 * HOUR);
        assert!(
            spoken.as_ref().is_none_or(|said| !said.line.contains("hour")),
            "twenty hours of history is not a rhythm: {spoken:?}",
        );

        // Enough days, but too few posts to have carved a histogram.
        let sparse: Vec<_> = (0..4).map(|day| post(START + day * 7 * DAY, "a note")).collect();
        let spoken = said(&sparse, START + 21 * DAY + HOUR);
        assert!(
            spoken.as_ref().is_none_or(|said| !said.line.contains("hour")),
            "four posts is not a histogram: {spoken:?}",
        );

        // A month of nights, and now it is entitled to an opinion. Posting at
        // noon is out of character for this archive and it says so.
        let mut nocturnal: Vec<_> = (0..30)
            .map(|day| post(START + day * DAY + 3 * HOUR, "another late one"))
            .collect();
        nocturnal.push(post(START + 30 * DAY + 12 * HOUR, "up early for once"));

        let spoken = said(&nocturnal, START + 30 * DAY + 13 * HOUR).expect("noon is news here");
        assert!(spoken.line.contains("an hour it does not keep"), "{}", spoken.line);
    }

    #[test]
    fn a_long_silence_is_the_thing_worth_saying() {
        let posts: Vec<_> = (0..20).map(|day| post(START + day * DAY, "a note")).collect();

        let spoken = said(&posts, START + 19 * DAY + 9 * DAY).expect("nine days is news");
        assert!(spoken.line.contains("not been fed"), "{}", spoken.line);
        assert!(spoken.bits >= MIN_BITS);
    }

    #[test]
    fn a_burst_is_the_thing_worth_saying() {
        // Twenty days of a single note, then six in ten minutes.
        let mut posts: Vec<_> = (0..20).map(|day| post(START + day * DAY, "a note")).collect();
        let burst = START + 20 * DAY;
        posts.extend((0..6).map(|i| post(burst + i * 2 * 60_000, "and another")));

        let spoken = said(&posts, burst + 12 * 60_000).expect("six at once is news");
        assert!(spoken.line.contains("single sitting"), "{}", spoken.line);
    }

    #[test]
    fn nothing_is_said_about_a_post_from_weeks_ago() {
        // A burst, read long after it. The burst is over; what is true now is the
        // silence, and that is what gets said.
        let mut posts: Vec<_> = (0..20).map(|day| post(START + day * DAY, "a note")).collect();
        let burst = START + 20 * DAY;
        posts.extend((0..6).map(|i| post(burst + i * 2 * 60_000, "and another")));

        let spoken = said(&posts, burst + 20 * DAY).expect("twenty days is news");
        assert!(!spoken.line.contains("sitting"), "{}", spoken.line);
        assert!(spoken.line.contains("not been fed"), "{}", spoken.line);
    }

    #[test]
    fn a_long_post_is_only_long_next_to_this_writers_own() {
        let long = ["word"; 120].join(" ");

        // Someone who writes at length anyway: another long one is a Tuesday.
        let mut prolix: Vec<_> = (0..20).map(|day| post(START + day * DAY, &long)).collect();
        prolix.push(post(START + 20 * DAY, &long));
        let spoken = said(&prolix, START + 20 * DAY + HOUR);
        assert!(
            spoken.as_ref().is_none_or(|said| !said.line.contains("something long")),
            "{spoken:?}",
        );

        // Someone who writes in fragments: the same post is unlike anything.
        let mut terse: Vec<_> = (0..20).map(|day| post(START + day * DAY, "a note")).collect();
        terse.push(post(START + 20 * DAY, &long));
        let spoken = said(&terse, START + 20 * DAY + HOUR).expect("that is a monologue");
        assert!(spoken.line.contains("something long"), "{}", spoken.line);
    }

    #[test]
    fn a_rare_feeling_is_worth_saying_and_a_common_one_is_not() {
        // Nineteen ordinary notes and one that is not.
        let mut posts: Vec<_> = (0..19)
            .map(|day| post_feeling(START + day * DAY, "a note", Mood::Content))
            .collect();
        posts.push(post_feeling(START + 19 * DAY, "a note", Mood::Melancholy));

        let spoken = said(&posts, START + 19 * DAY + HOUR).expect("one in twenty is news");
        assert!(spoken.line.contains("seldom felt this melancholy"), "{}", spoken.line);

        // The same archive where melancholy is simply how it always is.
        let usual: Vec<_> = (0..20)
            .map(|day| post_feeling(START + day * DAY, "a note", Mood::Melancholy))
            .collect();
        let spoken = said(&usual, START + 19 * DAY + HOUR);
        assert!(
            spoken.as_ref().is_none_or(|said| !said.line.contains("seldom")),
            "{spoken:?}",
        );
    }

    #[test]
    fn nothing_the_pet_says_addresses_the_reader() {
        // The public widget is read by strangers, so a line in the second person
        // would be telling them about someone else's habits. This is the rule that
        // erodes one convenient phrasing at a time, so it is pinned rather than
        // trusted.
        let mut spoken: Vec<String> = Vec::new();

        let mut archives: Vec<Vec<Morsel>> = vec![
            (0..20).map(|day| post(START + day * DAY, "a note")).collect(),
            run(START, 30, "shipped the rust deploy"),
            (0..19)
                .map(|day| post_feeling(START + day * DAY, "a note", Mood::Content))
                .chain([post_feeling(START + 19 * DAY, "wow", Mood::Chaos)])
                .collect(),
        ];
        archives.push({
            let mut burst: Vec<_> = (0..20).map(|day| post(START + day * DAY, "a note")).collect();
            burst.extend((0..7).map(|i| post(START + 20 * DAY + i * 60_000, &["word"; 200].join(" "))));
            burst
        });

        for archive in &archives {
            let last = archive.last().expect("posts").created_at;
            for offset in [HOUR, DAY, 5 * DAY, 30 * DAY, 200 * DAY] {
                if let Some(said) = said(archive, last + offset) {
                    spoken.push(said.line);
                }
            }
        }

        assert!(!spoken.is_empty(), "the corpus has to actually say something");
        for line in &spoken {
            let lowered = line.to_lowercase();
            for banned in [" you ", " you.", " your ", "you're", "yours"] {
                assert!(!lowered.contains(banned), "{line:?} addresses the reader");
            }
            assert!(lowered.starts_with(char::is_alphanumeric), "{line:?}");
            assert!(line.ends_with('.'), "{line:?} is not a sentence");
        }
    }

    #[test]
    fn the_same_moment_always_says_the_same_thing() {
        // The snapshot is shared by the widget, the sheet and the composer, and a
        // pet that contradicts itself between two panels on one page is worse than
        // one that repeats itself.
        let posts: Vec<_> = (0..20).map(|day| post(START + day * DAY, "a note")).collect();
        let at = START + 19 * DAY + 9 * DAY;

        let once = said(&posts, at);
        for _ in 0..50 {
            assert_eq!(said(&posts, at), once);
        }
    }

    #[test]
    fn a_tie_at_the_ceiling_is_shared_out_across_days() {
        // Two candidates both pinned at the cap would otherwise mean one of them
        // never gets said. Over a fortnight of equally-surprising days, more than
        // one line should appear.
        let mut posts: Vec<_> = (0..20).map(|day| post(START + day * DAY, "a note")).collect();
        posts.extend((0..8).map(|i| post(START + 20 * DAY + i * 60_000, &["word"; 200].join(" "))));

        let last = posts.last().expect("posts").created_at;
        let lines: std::collections::BTreeSet<String> = (0..14)
            .filter_map(|day| said(&posts, last + day * DAY).map(|said| said.line))
            .collect();

        assert!(lines.len() > 1, "one line monopolised the fortnight: {lines:?}");
    }

    #[test]
    fn surprise_is_capped_rather_than_running_away() {
        assert_eq!(bits(1.0), 0.0);
        assert_eq!(bits(0.5), 1.0);
        assert_eq!(bits(0.25), 2.0);
        assert_eq!(bits(1.0 / 17.0).min(MAX_BITS), MAX_BITS);
        assert_eq!(bits(0.0), MAX_BITS, "an impossible tail cannot arrive, but says so");
    }

    #[test]
    fn a_span_reads_in_the_largest_unit_that_stays_whole() {
        assert_eq!(duration(1.0), "an hour");
        assert_eq!(duration(1.9), "an hour");
        assert_eq!(duration(6.0), "6 hours");
        assert_eq!(duration(47.0), "47 hours");
        assert_eq!(duration(48.0), "2 days");
        assert_eq!(duration(13.0 * 24.0), "13 days");
        assert_eq!(duration(21.0 * 24.0), "3 weeks");
    }
}
