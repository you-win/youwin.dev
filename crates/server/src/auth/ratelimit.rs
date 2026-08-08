//! Per-IP login throttling.
//!
//! In-process: single binary, single process, so a `Mutex<HashMap>` is the whole
//! implementation. It resets on restart, which is acceptable — the threat is an
//! online guessing run, and a restart is not something an attacker can trigger.

use std::{
    collections::HashMap,
    sync::Mutex,
};

/// Failures tolerated before the first lockout.
const THRESHOLD: u32 = 5;

/// First lockout duration; doubles with each further failure.
const BASE_LOCKOUT_MILLIS: i64 = 15 * 60 * 1000;

/// Twelve hours. Without a ceiling, a handful of extra attempts pushes the
/// lockout past any plausible lifetime — and I would be the one locked out.
const MAX_LOCKOUT_MILLIS: i64 = 12 * 60 * 60 * 1000;

/// Entries older than this are discarded during pruning.
const IDLE_EVICT_MILLIS: i64 = 24 * 60 * 60 * 1000;

/// Above this many tracked keys, prune before inserting. Bounds memory against
/// a distributed run that would otherwise grow the map one entry per source.
const PRUNE_THRESHOLD: usize = 1024;

#[derive(Debug, Clone, Copy)]
struct Entry {
    failures: u32,
    locked_until: i64,
    last_seen: i64,
}

#[derive(Default)]
pub struct LoginLimiter {
    entries: Mutex<HashMap<String, Entry>>,
}

impl LoginLimiter {
    /// `Err(seconds)` when the caller is locked out, for a `Retry-After` header.
    pub fn check(&self, key: &str, now: i64) -> Result<(), i64> {
        let entries = self.entries.lock().expect("limiter mutex poisoned");

        match entries.get(key) {
            Some(entry) if entry.locked_until > now => {
                Err((entry.locked_until - now).div_euclid(1000).max(1))
            }
            _ => Ok(()),
        }
    }

    pub fn record_failure(&self, key: &str, now: i64) {
        let mut entries = self.entries.lock().expect("limiter mutex poisoned");

        if entries.len() > PRUNE_THRESHOLD {
            entries.retain(|_, e| now - e.last_seen < IDLE_EVICT_MILLIS || e.locked_until > now);
        }

        let entry = entries.entry(key.to_owned()).or_insert(Entry {
            failures: 0,
            locked_until: 0,
            last_seen: now,
        });

        entry.failures += 1;
        entry.last_seen = now;

        if entry.failures >= THRESHOLD {
            let doublings = (entry.failures - THRESHOLD).min(20);
            let multiplier = 2_i64.checked_pow(doublings).unwrap_or(i64::MAX);
            let lockout = BASE_LOCKOUT_MILLIS
                .saturating_mul(multiplier)
                .min(MAX_LOCKOUT_MILLIS);
            entry.locked_until = now.saturating_add(lockout);
        }
    }

    /// Called on a successful login: a correct password clears the record, so a
    /// few fumbled attempts followed by the right one costs nothing.
    pub fn reset(&self, key: &str) {
        self.entries
            .lock()
            .expect("limiter mutex poisoned")
            .remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_786_000_000_000;

    #[test]
    fn allows_attempts_up_to_the_threshold_then_locks() {
        let limiter = LoginLimiter::default();

        for _ in 0..THRESHOLD - 1 {
            limiter.record_failure("ip", NOW);
            assert!(limiter.check("ip", NOW).is_ok(), "must not lock early");
        }

        limiter.record_failure("ip", NOW);
        let retry = limiter.check("ip", NOW).expect_err("locked after threshold");
        assert_eq!(retry, BASE_LOCKOUT_MILLIS / 1000);
    }

    #[test]
    fn the_lockout_expires_on_its_own() {
        let limiter = LoginLimiter::default();
        for _ in 0..THRESHOLD {
            limiter.record_failure("ip", NOW);
        }

        assert!(limiter.check("ip", NOW + BASE_LOCKOUT_MILLIS - 1).is_err());
        assert!(limiter.check("ip", NOW + BASE_LOCKOUT_MILLIS + 1).is_ok());
    }

    #[test]
    fn further_failures_double_the_lockout_up_to_a_ceiling() {
        let limiter = LoginLimiter::default();
        for _ in 0..THRESHOLD + 1 {
            limiter.record_failure("ip", NOW);
        }
        assert_eq!(
            limiter.check("ip", NOW).unwrap_err(),
            2 * BASE_LOCKOUT_MILLIS / 1000
        );

        // Hammering must not push the lockout past the ceiling — otherwise the
        // person locked out for a year is me.
        for _ in 0..200 {
            limiter.record_failure("ip", NOW);
        }
        assert_eq!(
            limiter.check("ip", NOW).unwrap_err(),
            MAX_LOCKOUT_MILLIS / 1000
        );
    }

    #[test]
    fn a_successful_login_clears_the_record() {
        let limiter = LoginLimiter::default();
        for _ in 0..THRESHOLD {
            limiter.record_failure("ip", NOW);
        }
        assert!(limiter.check("ip", NOW).is_err());

        limiter.reset("ip");
        assert!(limiter.check("ip", NOW).is_ok());
    }

    #[test]
    fn lockouts_are_per_key() {
        let limiter = LoginLimiter::default();
        for _ in 0..THRESHOLD {
            limiter.record_failure("attacker", NOW);
        }

        assert!(limiter.check("attacker", NOW).is_err());
        assert!(limiter.check("me", NOW).is_ok(), "one bad IP must not lock others out");
    }

    #[test]
    fn idle_entries_are_pruned_so_the_map_stays_bounded() {
        let limiter = LoginLimiter::default();

        for i in 0..PRUNE_THRESHOLD + 2 {
            limiter.record_failure(&format!("ip-{i}"), NOW);
        }

        // A day later, one more failure triggers the prune of everything idle.
        let later = NOW + IDLE_EVICT_MILLIS + 1;
        limiter.record_failure("fresh", later);

        let entries = limiter.entries.lock().unwrap();
        assert!(
            entries.len() < PRUNE_THRESHOLD,
            "expected pruning, still holding {}",
            entries.len()
        );
        assert!(entries.contains_key("fresh"));
    }
}
