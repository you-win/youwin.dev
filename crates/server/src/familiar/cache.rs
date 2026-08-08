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

use std::sync::{Mutex, PoisonError};

use sqlx::SqlitePool;

use crate::{
    db,
    familiar::{
        Blend, Mood, PetState, compute, mood,
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
        let previous = match self.snapshot() {
            Some(held) if held.is_fresh(now) => return Ok(held),
            held => held.map(|held| held.state),
        };

        let posts = db::familiar::all(pool).await?;
        let state = compute(&posts, previous.as_ref(), now);
        let vitals = stats::vitals(&posts, now);
        let diet = topics::classify(&posts);

        let reading = Reading {
            sheet: stats::sheet(&state, &vitals, diet),
            state,
            vitals,
            diet,
            moods: mood::distribution(&posts),
        };

        self.store(&reading);
        Ok(reading)
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
        }
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
        let posts = run(START, 12, "rust deploy config");
        let state = compute(&posts, None, at);
        let vitals = stats::vitals(&posts, at);
        let diet = topics::classify(&posts);
        Reading {
            sheet: stats::sheet(&state, &vitals, diet),
            state,
            vitals,
            diet,
            moods: mood::distribution(&posts),
        }
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
    fn a_reading_agrees_with_the_state_it_holds() {
        let reading = reading_at(START + 11 * HOUR);

        assert_eq!(reading.state.posts, reading.vitals.posts);
        assert_eq!(reading.state.stage, Stage::Juvenile);
        assert_eq!(reading.sheet.vitality, stats::percent(reading.state.energy));
        assert!(matches!(reading.state.level, Level::Hyper | Level::Active));
    }
}
