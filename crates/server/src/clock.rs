//! Wall-clock time, in one place.
//!
//! Everything stored or compared is unix millis UTC. Centralised so that when
//! something needs a fake clock for a test, there is one function to intercept.

use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_millis() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is before 1970")
            .as_millis(),
    )
    .expect("timestamp fits in i64 until the year 292 million")
}
