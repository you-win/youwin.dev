//! Energy over time, and the circadian rhythm it is measured against.
//!
//! Two things live here because they are two halves of the same number. Energy
//! decays from the last post and jumps when new ones land; phase says whether
//! this is an hour the writer is usually awake for. The first is state that
//! carries forward, the second is a read-only offset applied at render time —
//! see [`step`].

use std::array;

use crate::familiar::{
    Morsel, PetState, Phase,
    baseline::{Baseline, FALLBACK_GAP_HOURS},
};

/// The pet never fully dies. At the floor it is a bare `( · · )`, which is a
/// state you can come back from; nothing renders as absent.
pub const FLOOR: f64 = 0.05;
pub const CEILING: f64 = 1.0;

/// Where a cold start begins, before decaying by the last post's age. Chosen so
/// a blog whose last post was an hour ago opens at "active" rather than
/// announcing itself as either exhausted or delighted.
const COLD_START_ENERGY: f64 = 0.6;

/// What an archive with no posts at all shows. Only ever seen by an egg.
const EMPTY_ENERGY: f64 = 0.5;

/// Half-lives per long gap.
///
/// The half-life is a multiple of the gap the writer exceeds a quarter of the
/// time (see [`Baseline::long_gap_hours`]), so at two, a silence has to run to
/// about twice one of their longer stretches before the pet is down to half. That
/// is roughly where a quiet spell stops looking like a quiet spell.
const HALF_LIFE_FACTOR: f64 = 2.0;

/// However fast someone posts, energy does not evaporate quicker than this. The
/// shortest measurable gap between sittings is 45 minutes, so without a floor a
/// pet could be built that visibly sagged inside one.
const MIN_HALF_LIFE_HOURS: f64 = 2.0;

/// However slowly someone posts, the pet's patience ends somewhere.
///
/// Without a ceiling the formula is perfectly happy to conclude that a blog with
/// two posts a year apart is on schedule six months later, which is true and
/// useless: nothing about that pet would ever move. A fortnight is the pet's
/// memory. Beyond that it stops pretending, and the writers this bites are the
/// monthly ones, who were never the audience for a creature that changes daily.
const MAX_HALF_LIFE_HOURS: f64 = 14.0 * 24.0;

/// Days over which the learned posting rhythm displaces the assumed one.
///
/// Public because it is also the point at which [`super::speech`] is willing to
/// say anything out loud about the hours this archive keeps: before it, the
/// profile is still partly a guess about a writer nobody has watched yet.
pub const COLD_START_DAYS: f64 = 14.0;

/// Window over which posts stack into a burst.
const BURST_WINDOW_MINUTES: f64 = 30.0;

/// What one post is worth, before the recency and cadence factors.
const BURST_PER_POST: f64 = 0.20;

/// Ceiling on a single gap's boost, so a spam run cannot pin the pet at hyper.
const MAX_BURST: f64 = 0.7;

/// The distance credited to the first post in a gap.
///
/// Not the real interval to whatever came before it: a post after three days of
/// silence must still wake the pet up, and measuring the true gap there would
/// give it a boost of essentially zero — the one case where a boost matters most.
/// Every post is therefore worth a full wake-up, and *closeness to the post
/// before it* is what stacks on top.
const FIRST_POST_MINUTES: f64 = 1.0;

const HOUR_MILLIS: i64 = 3_600_000;

/// Advances energy to `now`, without any circadian offset.
///
/// Decay first, then boosts from posts written since the last computation. The
/// result is what carries forward to the next call, which is precisely why the
/// phase modifier is *not* folded in here: this value is recomputed on every
/// cache miss, so anything added to it compounds once per page load. A pet whose
/// energy rose 0.10 per visit during peak hours could be walked to hyper by
/// holding down F5, and one visited through the night would be driven to the
/// floor by traffic rather than by silence. [`super::compute`] adds the offset to
/// the copy it renders and throws it away.
pub fn step(posts: &[Morsel], previous: Option<&PetState>, now: i64) -> f64 {
    let Some(last) = posts.last() else {
        return EMPTY_ENERGY;
    };

    let rhythm = Baseline::of(posts);
    let half_life = (rhythm.long_gap_hours() * HALF_LIFE_FACTOR)
        .clamp(MIN_HALF_LIFE_HOURS, MAX_HALF_LIFE_HOURS);

    // No previous state: a fresh process, or the first visit after a restart.
    // Estimating from the last post's age rather than starting at zero is what
    // makes a redeploy invisible — the pet comes back roughly where it was.
    let Some(previous) = previous else {
        return decay(COLD_START_ENERGY, hours(now - last.created_at), half_life);
    };

    let carried = decay(previous.base_energy, hours(now - previous.at), half_life);
    let since = posts.partition_point(|post| post.created_at <= previous.at);

    (carried + burst(&posts[since..], rhythm.typical_gap_hours())).clamp(FLOOR, CEILING)
}

/// `E(t) = E₀ × 2^(-t / τ)`, floored.
///
/// Exponential because the shape is right: a gap the writer would not notice
/// barely registers, a few of them are felt, and an absence is terminal — with a
/// long tail rather than a cliff. `τ` comes from the writer's own distribution of
/// gaps rather than from the clock, so the same curve serves someone who writes
/// hourly and someone who writes on Sundays.
fn decay(energy: f64, elapsed_hours: f64, half_life_hours: f64) -> f64 {
    if elapsed_hours <= 0.0 {
        return energy;
    }
    (energy * 2f64.powf(-elapsed_hours / half_life_hours)).max(FLOOR)
}

/// Amplification bounds for the cadence factor. A burst is worth twice as much
/// to someone who writes daily as to someone writing every six hours, and half
/// as much to someone writing constantly — but never more or less than that, or
/// a quiet fortnight would make the next post an event.
const MIN_CADENCE_FACTOR: f64 = 0.5;
const MAX_CADENCE_FACTOR: f64 = 2.0;

/// The boost from posts written during one gap.
///
/// Additive across posts and scaled by how far apart they are, so three notes in
/// ten minutes lift the pet noticeably further than three spread across a day.
///
/// The cadence factor amplifies the whole thing **for slower writers**: three
/// posts from someone who manages one a day is a burst, and the same three from
/// someone who writes hourly is a Tuesday. The prototype's factor runs the other
/// way — `6 / cadence`, which rewards the fast writer — against the stated
/// intent in both its own docstring and the design's energy section. This is the
/// version those two describe.
///
/// `typical_gap_hours` is the gap between *sittings*, so the writer who posts
/// five times every Sunday is now correctly read as the slow writer they are and
/// their Sunday counts for the most this allows, rather than as a frantic one
/// whose every note is unremarkable.
fn burst(gap: &[Morsel], typical_gap_hours: f64) -> f64 {
    if gap.is_empty() {
        return 0.0;
    }

    let mut total = 0.0;
    let mut previous: Option<i64> = None;

    for post in gap {
        let minutes = match previous {
            None => FIRST_POST_MINUTES,
            Some(before) => minutes(post.created_at - before).max(0.1),
        };
        total += BURST_PER_POST * (-minutes / BURST_WINDOW_MINUTES).exp();
        previous = Some(post.created_at);
    }

    let cadence_factor =
        (typical_gap_hours / FALLBACK_GAP_HOURS).clamp(MIN_CADENCE_FACTOR, MAX_CADENCE_FACTOR);
    (total * cadence_factor).min(MAX_BURST)
}

/// Which phase `now` falls in, given everything written so far.
pub fn phase_at(posts: &[Morsel], now: i64) -> Phase {
    phases(&profile(posts, now))[hour_of(now)]
}

/// The hourly posting profile the phases are cut from. Densities summing to one.
///
/// Public because [`super::speech`] judges the hour a post landed in against the
/// same curve the phase is cut from — two answers to "is this a normal time for
/// this archive" that must not be able to disagree.
///
/// Blends the learned histogram with an assumed human schedule, weighted by how
/// much history there is: nothing on day one, entirely learned after
/// [`COLD_START_DAYS`]. The design asks for a default schedule that "gradually
/// shifts to match the user's actual pattern", and a weighted blend is that
/// sentence rather than an approximation of it — no threshold at which the pet's
/// sense of time lurches.
pub fn profile(posts: &[Morsel], now: i64) -> [f64; 24] {
    let learned = normalized(smoothed(histogram(posts)));
    let assumed = normalized(ASSUMED_SCHEDULE);

    let age_days = posts
        .first()
        .map_or(0.0, |first| hours(now - first.created_at) / 24.0);
    let learned_share = (age_days / COLD_START_DAYS).clamp(0.0, 1.0);

    array::from_fn(|hour| {
        (1.0 - learned_share) * assumed[hour] + learned_share * learned[hour]
    })
}

/// The schedule assumed before the archive has taught the pet its own.
///
/// A broad bump centred on 11:00 **UTC** — the site renders every timestamp in
/// UTC and does not guess at zones (see `view::time_fmt`), so this is a guess
/// about the writer's, and a wrong one is corrected within a fortnight by the
/// blend above. Everything downstream is relative to this curve's own peak, so
/// once the histogram takes over the absolute hours stop mattering at all.
const ASSUMED_SCHEDULE: [f64; 24] = {
    let mut hours = [0.0; 24];
    let mut hour: usize = 0;
    while hour < 24 {
        // Centred on 11, so the furthest hour is 23 at a distance of 12 — half
        // the circle. Distance around the clock and distance along it agree
        // everywhere, and there is no wrap to handle.
        let distance = hour.abs_diff(11);
        // exp(-d² / 2σ²) with σ = 3, unrolled: `exp` is not const.
        hours[hour] = match distance {
            0 => 1.0,
            1 => 0.9460,
            2 => 0.8007,
            3 => 0.6065,
            4 => 0.4111,
            5 => 0.2494,
            6 => 0.1353,
            7 => 0.0657,
            8 => 0.0286,
            9 => 0.0111,
            10 => 0.0039,
            11 => 0.0012,
            _ => 0.0003,
        };
        hour += 1;
    }
    hours
};

/// Raw posts-per-hour counts, UTC.
fn histogram(posts: &[Morsel]) -> [f64; 24] {
    let mut counts = [0.0; 24];
    for post in posts {
        counts[hour_of(post.created_at)] += 1.0;
    }
    counts
}

/// Gaussian smoothing, σ = 1 hour, truncated at ±2 and wrapped around midnight.
///
/// Without it a single 3 AM post carves a spike into the profile and can drag
/// the peak window with it. With it, one stray night counts for little and a
/// habit of them counts for a lot.
fn smoothed(raw: [f64; 24]) -> [f64; 24] {
    const KERNEL: [f64; 5] = [0.1353, 0.6065, 1.0, 0.6065, 0.1353];
    let weight: f64 = KERNEL.iter().sum();

    array::from_fn(|hour| {
        KERNEL
            .iter()
            .enumerate()
            .map(|(index, k)| {
                // index 0..=4 maps to offsets -2..=2.
                let at = (hour + 24 + index - 2) % 24;
                raw[at] * k
            })
            .sum::<f64>()
            / weight
    })
}

fn normalized(profile: [f64; 24]) -> [f64; 24] {
    let total: f64 = profile.iter().sum();
    if total <= 0.0 {
        return [0.0; 24];
    }
    profile.map(|density| density / total)
}

/// Cuts a profile into the five phases.
///
/// The densest four-hour block is `peak` — four because it is about the width of
/// one sitting, and because narrower windows chase noise while wider ones stop
/// meaning anything. Everything else is judged relative to that block's density.
fn phases(profile: &[f64; 24]) -> [Phase; 24] {
    let mut best_start = 0;
    let mut best_total = f64::NEG_INFINITY;
    for start in 0..24 {
        let total: f64 = (0..4).map(|offset| profile[(start + offset) % 24]).sum();
        if total > best_total {
            best_total = total;
            best_start = start;
        }
    }

    let peak_mean = best_total / 4.0;
    let is_peak = |hour: usize| (0..4).any(|offset| (best_start + offset) % 24 == hour);

    let mut phases = [Phase::Deep; 24];
    for (hour, phase) in phases.iter_mut().enumerate() {
        *phase = if is_peak(hour) {
            Phase::Peak
        } else if profile[hour] >= peak_mean * 0.50 {
            Phase::Active
        } else {
            Phase::Deep
        };
    }

    // Transition zones, in a second pass so an hour beside the *original* awake
    // block becomes dusk rather than dusk begetting more dusk.
    //
    // Measured around the circle rather than between the first and last awake
    // hour. Taking a min and a max works for a nine-to-five and silently breaks
    // for a night owl, whose awake hours straddle midnight: the earliest is 0
    // and the latest 23, the two hours "outside" that range are awake ones, and
    // the pet gets no dusk at all.
    let awake = phases;
    for hour in 0..24 {
        if matches!(awake[hour], Phase::Peak | Phase::Active) {
            continue;
        }
        let beside_awake = (1..=2).any(|distance| {
            matches!(awake[(hour + distance) % 24], Phase::Peak | Phase::Active)
                || matches!(awake[(hour + 24 - distance) % 24], Phase::Peak | Phase::Active)
        });

        phases[hour] = if beside_awake {
            Phase::Dusk
        } else if profile[hour] >= peak_mean * 0.20 {
            Phase::Quiet
        } else {
            Phase::Deep
        };
    }

    phases
}

/// Hour of day, UTC.
///
/// Integer division rather than a calendar library: unix time counts elapsed
/// seconds with no leap seconds and its epoch is midnight UTC, so the hour is
/// exact arithmetic. `div_euclid` keeps that true for timestamps before 1970,
/// which a corrupt row could hold.
pub fn hour_of(millis: i64) -> usize {
    millis.div_euclid(HOUR_MILLIS).rem_euclid(24) as usize
}

fn hours(millis: i64) -> f64 {
    millis as f64 / HOUR_MILLIS as f64
}

fn minutes(millis: i64) -> f64 {
    millis as f64 / 60_000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::familiar::{
        Level, compute,
        fixture::{DAY, HOUR, START, post, run},
    };

    #[test]
    fn the_epoch_hour_is_exact_in_both_directions() {
        assert_eq!(hour_of(START), 0, "the fixture starts at midnight UTC");
        assert_eq!(hour_of(START + 13 * HOUR), 13);
        assert_eq!(hour_of(START + 25 * HOUR), 1);
        assert_eq!(hour_of(-HOUR), 23, "an hour before the epoch is 23:00");
    }

    #[test]
    fn energy_halves_over_one_half_life() {
        assert!((decay(0.8, 6.0, 6.0) - 0.4).abs() < 1e-9);
        assert!((decay(0.8, 12.0, 6.0) - 0.2).abs() < 1e-9);
        // Never below the floor, however long the silence.
        assert_eq!(decay(0.8, 10_000.0, 6.0), FLOOR);
        // Time never runs backwards for the pet.
        assert_eq!(decay(0.8, -5.0, 6.0), 0.8);
    }

    #[test]
    fn a_weekly_writer_is_not_mistaken_for_an_absent_one() {
        // The regression this whole module was built for. Five notes every Sunday
        // measured, under the old mean gap between posts, as a cadence of five
        // minutes — a two-hour half-life, and a pet flat on the floor from Monday
        // until the next weekend, for someone doing exactly what they always did.
        let posts: Vec<_> = (0..8)
            .flat_map(|week| {
                (0..5).map(move |note| {
                    post(START + week * 7 * DAY + 10 * HOUR + note * 5 * 60_000, "the weekly note")
                })
            })
            .collect();
        let last = posts.last().expect("posts").created_at;

        // Midweek: quiet, but this is what their week looks like.
        let midweek = compute(&posts, None, last + 3 * DAY + 12 * HOUR);
        assert!(
            midweek.base_energy > 0.30,
            "a normal Wednesday read as {}",
            midweek.base_energy,
        );

        // Three weeks of nothing is not what their week looks like, and the same
        // curve says so without a second rule.
        //
        // Read off `base_energy`, not `level`: level carries the circadian offset
        // too, and both of these fall in the hours this archive is written in, so
        // asserting on it would be testing the phase curve rather than the
        // half-life this test is about.
        let absent = compute(&posts, None, last + 21 * DAY);
        assert!(
            matches!(Level::of(absent.base_energy), Level::Lethargic | Level::Bored),
            "three weeks away read as {:?} ({})",
            Level::of(absent.base_energy),
            absent.base_energy,
        );
    }

    #[test]
    fn the_half_life_is_bounded_at_both_ends() {
        // A yearly writer is on schedule for months at a time, which is true and
        // would leave a pet that never moved. The ceiling is what stops the
        // formula from concluding it.
        let yearly: Vec<_> = (0..4)
            .map(|year| post(START + year * 365 * DAY, "the annual update"))
            .collect();
        let rhythm = Baseline::of(&yearly);
        assert!(rhythm.long_gap_hours() * HALF_LIFE_FACTOR > MAX_HALF_LIFE_HOURS);

        let stale = compute(&yearly, None, START + 3 * 365 * DAY + 180 * DAY);
        assert_eq!(stale.base_energy, FLOOR, "half a year on and still going");

        // And nothing posts fast enough to sag inside a single sitting.
        let rapid: Vec<_> = (0..20)
            .map(|i| post(START + i * 46 * 60_000, "another"))
            .collect();
        let tight = Baseline::of(&rapid);
        assert!(tight.long_gap_hours() * HALF_LIFE_FACTOR < MIN_HALF_LIFE_HOURS);
    }

    #[test]
    fn a_burst_stacks_and_a_trickle_does_not() {
        let rapid = [
            post(START, "a"),
            post(START + 5 * 60_000, "b"),
            post(START + 10 * 60_000, "c"),
        ];
        let spread = [
            post(START, "a"),
            post(START + 4 * HOUR, "b"),
            post(START + 8 * HOUR, "c"),
        ];

        let tight = burst(&rapid, FALLBACK_GAP_HOURS);
        let loose = burst(&spread, FALLBACK_GAP_HOURS);

        assert!(tight > loose, "{tight} should beat {loose}");
        assert!(burst(&[], FALLBACK_GAP_HOURS) == 0.0);
        // A single post is still worth waking up for, however long the silence
        // before it.
        assert!(burst(&rapid[..1], FALLBACK_GAP_HOURS) > 0.15);
    }

    #[test]
    fn a_burst_cannot_pin_the_pet_at_hyper() {
        let spam: Vec<_> = (0..500)
            .map(|i| post(START + i * 1_000, "more"))
            .collect();
        assert!(burst(&spam, FALLBACK_GAP_HOURS) <= MAX_BURST);
    }

    #[test]
    fn a_slow_writers_burst_counts_for_more() {
        let three = [
            post(START, "a"),
            post(START + 5 * 60_000, "b"),
            post(START + 10 * 60_000, "c"),
        ];
        let daily = burst(&three, 24.0);
        let hourly = burst(&three, 1.0);
        assert!(daily > hourly, "daily {daily} should beat hourly {hourly}");
    }

    #[test]
    fn silence_walks_the_pet_down_to_the_floor() {
        let posts = run(START, 12, "rust deploy");
        let last = START + 11 * HOUR;

        let fresh = compute(&posts, None, last);
        assert!(
            matches!(fresh.level, Level::Hyper | Level::Active),
            "{:?}",
            fresh.level
        );

        // Three days later, with nothing written, and stepped forward one hour
        // at a time the way a stream of visitors would step it.
        let mut state = fresh;
        for step in 1..=(3 * 24) {
            state = compute(&posts, Some(&state), last + step * HOUR);
        }
        assert_eq!(state.base_energy, FLOOR, "nothing left to decay");

        // What that *reads* as still depends on the hour. In the middle of the
        // night the pet is plainly bored; during the hours it usually writes in,
        // the circadian lift keeps a flicker of lethargy — which is the point of
        // having a phase at all.
        let deep = compute(&posts, Some(&state), last + 3 * DAY + 12 * HOUR);
        assert_eq!(deep.phase, Phase::Deep, "hour {}", hour_of(deep.at));
        assert_eq!(deep.level, Level::Bored, "energy {}", deep.energy);
        assert!(deep.energy >= FLOOR);
    }

    #[test]
    fn a_new_post_wakes_a_flat_pet() {
        let mut posts = run(START, 12, "rust deploy");
        let quiet_for_days = START + 11 * HOUR + 4 * DAY;

        let asleep = compute(&posts, None, quiet_for_days);
        assert_eq!(asleep.base_energy, FLOOR);

        posts.push(post(quiet_for_days + HOUR, "back at it, one more deploy"));
        let woken = compute(&posts, Some(&asleep), quiet_for_days + HOUR);

        assert!(
            woken.base_energy > asleep.base_energy,
            "{} should exceed {}",
            woken.base_energy,
            asleep.base_energy
        );
    }

    #[test]
    fn a_restart_lands_where_the_pet_was() {
        // The cold-start path exists so a redeploy is invisible: a process that
        // has just come up has no carried energy, and estimating from the last
        // post's age has to put the pet back where the visitor left it.
        let posts = run(START, 20, "rust deploy config");
        let at = START + 20 * HOUR;

        let carried = compute(&posts, None, START + 19 * HOUR);
        let carried = compute(&posts, Some(&carried), at);
        let cold = compute(&posts, None, at);

        assert_eq!(cold.level, carried.level, "{} vs {}", cold.energy, carried.energy);
    }

    #[test]
    fn the_assumed_schedule_peaks_around_late_morning() {
        let phases = phases(&normalized(ASSUMED_SCHEDULE));
        assert_eq!(phases[11], Phase::Peak);
        assert_eq!(phases[10], Phase::Peak);
        assert_eq!(phases[3], Phase::Deep);
        assert_eq!(phases[22], Phase::Deep);
    }

    #[test]
    fn a_night_owl_gets_a_dusk_either_side_of_midnight() {
        // Awake hours straddling midnight are what breaks a min/max boundary
        // scan: there has to be a transition zone on both sides of the block.
        let mut profile = [0.0; 24];
        for hour in [22, 23, 0, 1, 2] {
            profile[hour] = 1.0;
        }

        let phases = phases(&normalized(smoothed(profile)));
        let awake: Vec<_> = (0..24)
            .filter(|h| matches!(phases[*h], Phase::Peak | Phase::Active))
            .collect();
        assert!(awake.contains(&23) && awake.contains(&0), "{phases:?}");

        let dusk: Vec<_> = (0..24).filter(|h| phases[*h] == Phase::Dusk).collect();
        assert!(!dusk.is_empty(), "a night owl still winds down: {phases:?}");
        assert!(
            dusk.iter().any(|h| *h < 12) && dusk.iter().any(|h| *h > 12),
            "dusk should appear on both sides of the block, got {dusk:?}",
        );
    }

    #[test]
    fn the_learned_rhythm_displaces_the_assumed_one() {
        // Someone who only ever posts at 03:00 UTC. On day one the pet still
        // believes in the default schedule; three weeks in, 03:00 is its peak.
        let posts: Vec<_> = (0..40)
            .map(|day| post(START + day * DAY + 3 * HOUR, "another late one"))
            .collect();

        let day_one = phase_at(&posts[..1], START + 3 * HOUR + 60_000);
        assert_eq!(day_one, Phase::Deep, "the assumed schedule is asleep at 3am");

        let weeks_later = phase_at(&posts, START + 39 * DAY + 3 * HOUR + 60_000);
        assert_eq!(weeks_later, Phase::Peak);
    }

    #[test]
    fn smoothing_spreads_a_single_night_without_erasing_a_habit() {
        let mut once = [0.0; 24];
        once[3] = 1.0;
        let spread = smoothed(once);
        assert!(spread[3] > spread[2] && spread[2] > spread[1]);
        assert!(spread[1] > 0.0, "±2 hours pick up some of it");
        assert_eq!(spread[12], 0.0, "and nothing further out does");

        // Wrapping works: 23:00 leaks into 00:00, not off the end of the array.
        let mut midnight = [0.0; 24];
        midnight[23] = 1.0;
        let wrapped = smoothed(midnight);
        assert!(wrapped[0] > 0.0 && wrapped[1] > 0.0);
    }
}
