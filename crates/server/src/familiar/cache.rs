//! The snapshot the public site actually serves, and how it catches up.
//!
//! The pet is a function of time, and the pages it appears on are cached at the
//! edge for five minutes. Recomputing it per request would be work nobody sees;
//! recomputing it never would freeze it. So it recomputes on the first request
//! after the snapshot goes stale, fast-forwarding from wherever it was to now.
//!
//! The useful consequence is that the pet reflects the gap between the *last
//! visit* and the *last post*, not just the last post. Through a quiet week
//! nothing runs at all; the next visitor triggers one catch-up and sees a pet
//! that has plainly been alone.

use std::sync::{
    Mutex, PoisonError,
    atomic::{AtomicBool, Ordering},
};

use sqlx::SqlitePool;

use crate::{
    db,
    familiar::{
        Baseline, Blend, Mood, Morsel, PetState, compute, mood,
        speech::{self, Utterance},
        stats::{self, Sheet, Vitals},
        topics,
    },
};

/// How long a snapshot is served before it is recomputed.
///
/// Matched to the `s-maxage=300` the public site is served with, so the pet goes
/// stale on the same beat as the page it is drawn on. A shorter TTL would
/// recompute for responses Cloudflare is answering from its own copy anyway.
pub const TTL_MILLIS: i64 = 5 * 60 * 1000;

/// Everything both views need, computed together.
///
/// One struct rather than a state plus four lazy getters: the feed widget and
/// the `/familiar` page are the same snapshot seen at two levels of detail, and
/// deriving the stats separately would mean the page could disagree with the
/// widget that linked to it.
#[derive(Debug, Clone)]
pub struct Reading {
    pub state: PetState,
    pub vitals: Vitals,
    /// The topic split across the whole archive — the pet's *form* is set by a
    /// rolling window instead, so this is the long view and they can differ.
    pub diet: Blend,
    /// Moods across the whole archive, commonest first.
    pub moods: Vec<(Mood, f64)>,
    pub sheet: Sheet,
    /// The one thing worth saying right now, or nothing on an ordinary day.
    ///
    /// Part of the snapshot rather than derived per surface, so the widget, the
    /// character sheet and the composer cannot say three different things about
    /// the same moment — and so the day-rotation between equally surprising
    /// candidates lands on one answer per reading.
    pub speech: Option<Utterance>,
}

impl Reading {
    /// Whether this snapshot still stands at `now`.
    ///
    /// A half-open window from zero, so a clock that has gone backwards — an
    /// NTP correction, a restored VM — counts as stale rather than as very
    /// fresh indeed.
    fn is_fresh(&self, now: i64) -> bool {
        (0..TTL_MILLIS).contains(&(now - self.state.at))
    }
}

/// Holds the snapshot. One per process, on the public listener's state.
///
/// The lock is recovered from poisoning rather than propagating it: a panic
/// while rendering one page must not leave the pet unreadable on every page
/// after it, and there is no invariant a half-written snapshot could break —
/// the value is replaced wholesale or not at all.
#[derive(Debug, Default)]
pub struct Familiar {
    held: Mutex<Option<Reading>>,
    /// Set by [`Familiar::forget`], cleared by the next recompute that lands.
    ///
    /// Separate from the snapshot itself because staleness and the energy the
    /// snapshot carries are two different things, and a write invalidates only
    /// the first. See [`Familiar::forget`].
    stale: AtomicBool,
}

impl Familiar {
    pub fn new() -> Self {
        Self::default()
    }

    /// The pet at `now`, from the snapshot when it is fresh and from the
    /// database when it is not.
    ///
    /// Two requests arriving together on a stale snapshot will both recompute.
    /// That is deliberate: the alternative is holding a lock across a query, so
    /// every page load on the site queues behind whichever one missed. The
    /// duplicated work is one read of a table this site keeps in page cache, at
    /// most once per TTL, and the later of the two writes wins — which is why
    /// the store below refuses to move the snapshot backwards.
    pub async fn read(&self, pool: &SqlitePool, now: i64) -> Result<Reading, sqlx::Error> {
        let stale = self.stale.load(Ordering::Relaxed);
        let previous = match self.snapshot() {
            Some(held) if !stale && held.is_fresh(now) => return Ok(held),
            held => held.map(|held| held.state),
        };

        let posts = db::familiar::all(pool).await?;
        let reading = reading(&posts, previous.as_ref(), now);

        self.store(&reading);
        Ok(reading)
    }

    /// The pet as it would be if `draft` were posted now.
    ///
    /// **Never stored.** A hypothetical is not a fact about the archive, and the
    /// composer asks this question on every pause in typing — a preview that
    /// wrote to the snapshot would leave the pet reflecting a post that was
    /// never made, until the TTL expired or the process restarted.
    ///
    /// The carried state still comes from the snapshot, so this answers "what
    /// would this do to the pet *as it is now*" rather than recomputing one from
    /// nothing and comparing two unrelated numbers.
    pub async fn with_draft(
        &self,
        pool: &SqlitePool,
        now: i64,
        draft: Morsel,
    ) -> Result<Reading, sqlx::Error> {
        let previous = self.snapshot().map(|held| held.state);
        let mut posts = db::familiar::all(pool).await?;

        // The draft lands strictly after two things, and both matter.
        //
        // After the carried state, or [`super::energy::step`] treats it as
        // already seen and the preview shows none of the burst it exists to show
        // — which happens whenever a recompute landed in this same millisecond.
        //
        // After the last real post, because [`compute`] binary-searches this
        // slice and a draft appended out of order would silently cut the archive
        // short. Nothing schedules posts, so this is insurance rather than a
        // case that arises, and it costs one comparison.
        let at = [
            now,
            previous.map_or(i64::MIN, |state| state.at + 1),
            posts.last().map_or(i64::MIN, |post| post.created_at + 1),
        ]
        .into_iter()
        .max()
        .expect("three elements");

        posts.push(Morsel {
            created_at: at,
            ..draft
        });

        Ok(reading(&posts, previous.as_ref(), at))
    }

    /// Marks the snapshot as needing recomputation, **without discarding the
    /// energy it is carrying**.
    ///
    /// Called after a write on the authoring host, where the pet's whole job is
    /// to show what the thing you just posted did to it — a five-minute wait for
    /// that is the TTL doing exactly the wrong thing. The public site's own
    /// snapshot is deliberately *not* invalidated: it is matched to the edge
    /// cache in front of it, so recomputing early would be work for a response
    /// Cloudflare is answering from its own copy anyway.
    ///
    /// The distinction between stale and absent is the whole of this method, and
    /// it is not a nicety. Dropping the `Reading` outright hands the next read a
    /// `previous` of `None`, which is the cold-start path — and cold start
    /// estimates from the last post's age and applies *no burst at all*. So the
    /// composer would preview a post as the jolt it genuinely is, the post would
    /// land, and the pet would settle at the flat cold-start value instead. The
    /// preview would be wrong about every post, in the one direction that makes
    /// the feature pointless.
    pub fn forget(&self) {
        self.stale.store(true, Ordering::Relaxed);
    }

    fn snapshot(&self) -> Option<Reading> {
        self.held
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn store(&self, reading: &Reading) {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);

        // A snapshot from further ahead in time already covers this one. Without
        // the guard, two concurrent recomputes could leave the older of the two
        // in place and hand the next fast-forward a negative interval to work
        // from — which decay treats as "no time passed" and quietly stalls the pet.
        if held.as_ref().is_none_or(|current| current.state.at <= reading.state.at) {
            *held = Some(reading.clone());
            self.stale.store(false, Ordering::Relaxed);
        }
    }
}

/// Everything both views need, from one slice of posts.
///
/// A free function rather than a method: it is the whole of what a [`Reading`]
/// *is*, and both the real read and the draft preview have to build it the same
/// way or the two would disagree about what a post does.
fn reading(posts: &[Morsel], previous: Option<&PetState>, now: i64) -> Reading {
    let state = compute(posts, previous, now);
    let vitals = stats::vitals(posts, now);
    let diet = topics::classify(posts);
    let moods = mood::distribution(posts);

    Reading {
        sheet: stats::sheet(&state, &vitals, diet),
        speech: speech::speak(posts, &Baseline::of(posts), &state, &moods, now),
        state,
        vitals,
        diet,
        moods,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::familiar::{
        Level, Stage,
        fixture::{HOUR, START, run},
    };

    /// The cache's query is covered by `tests/familiar.rs` against a real pool.
    /// These exercise the freshness and ordering rules directly, which is the
    /// part that has edge cases.
    fn reading_at(at: i64) -> Reading {
        reading(&run(START, 12, "rust deploy config"), None, at)
    }

    #[test]
    fn freshness_is_a_half_open_window_that_a_backwards_clock_falls_out_of() {
        let held = reading_at(START);

        assert!(held.is_fresh(START));
        assert!(held.is_fresh(START + TTL_MILLIS - 1));
        assert!(!held.is_fresh(START + TTL_MILLIS), "the window is half open");
        assert!(!held.is_fresh(START - 1), "a clock that jumped back is stale");
    }

    #[test]
    fn the_snapshot_never_moves_backwards() {
        let familiar = Familiar::new();

        familiar.store(&reading_at(START + 2 * HOUR));
        familiar.store(&reading_at(START + HOUR));

        assert_eq!(
            familiar.snapshot().expect("held").state.at,
            START + 2 * HOUR,
            "the earlier recompute must not overwrite the later one",
        );
    }

    #[test]
    fn forgetting_expires_the_snapshot_without_throwing_away_its_energy() {
        let familiar = Familiar::new();
        familiar.store(&reading_at(START));

        familiar.forget();

        assert!(familiar.stale.load(Ordering::Relaxed), "the next read must recompute");

        // And the state survives it. Dropping the Reading outright would hand
        // the next read a `previous` of None — the cold-start path, which applies
        // no burst — so the composer would promise every post a jolt the post
        // itself then failed to deliver.
        let carried = familiar.snapshot().expect("the carried state must survive");
        assert_eq!(carried.state.at, START);

        // Recomputing clears it again.
        familiar.store(&reading_at(START + HOUR));
        assert!(!familiar.stale.load(Ordering::Relaxed));
    }

    #[test]
    fn a_reading_agrees_with_the_state_it_holds() {
        let reading = reading_at(START + 11 * HOUR);

        assert_eq!(reading.state.posts, reading.vitals.posts);
        assert_eq!(reading.state.stage, Stage::Juvenile);
        assert_eq!(reading.sheet.vitality, stats::percent(reading.state.energy));
        assert!(matches!(reading.state.level, Level::Hyper | Level::Active));
    }
}
