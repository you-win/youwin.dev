//! One test per statement in `db::sessions`.
//!
//! `create`, `lookup`, `delete`, `delete_all`, and `count_active` are also
//! exercised end-to-end in `tests/auth.rs`; they are covered again here at the
//! query level, because the HTTP tests would still pass if a statement returned
//! the right status for the wrong reason. `touch` and `purge_expired` are
//! reachable only from here — nothing in a single request triggers them.

use sqlx::SqlitePool;
use youwin_server::db::sessions;

const NOW: i64 = 1_786_000_000_000;
const HOUR: i64 = 60 * 60 * 1000;

fn token(seed: u8) -> Vec<u8> {
    vec![seed; 32]
}

#[sqlx::test]
async fn create_then_lookup_returns_the_stored_window(pool: SqlitePool) {
    sessions::create(
        &pool,
        &token(1),
        NOW,
        NOW + 24 * HOUR,
        Some("Firefox"),
        Some("203.0.113.7"),
    )
    .await
    .unwrap();

    let found = sessions::lookup(&pool, &token(1), NOW)
        .await
        .unwrap()
        .expect("live session");

    assert_eq!(found.created_at, NOW);
    assert_eq!(found.expires_at, NOW + 24 * HOUR);
    assert_eq!(found.last_seen_at, NOW, "last_seen starts at creation");

    // The metadata columns exist for the settings screen; a typo in the INSERT
    // would leave them null and nothing else would notice.
    let (ua, ip): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT user_agent, ip FROM sessions WHERE token_hash = ?1")
            .bind(token(1))
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(ua.as_deref(), Some("Firefox"));
    assert_eq!(ip.as_deref(), Some("203.0.113.7"));
}

#[sqlx::test]
async fn lookup_enforces_expiry_in_sql(pool: SqlitePool) {
    sessions::create(&pool, &token(1), NOW, NOW + HOUR, None, None)
        .await
        .unwrap();

    assert!(sessions::lookup(&pool, &token(1), NOW + HOUR - 1).await.unwrap().is_some());
    // Boundary: expires_at is exclusive, so exactly-at-expiry is already dead.
    assert!(sessions::lookup(&pool, &token(1), NOW + HOUR).await.unwrap().is_none());
    assert!(sessions::lookup(&pool, &token(1), NOW + HOUR + 1).await.unwrap().is_none());

    // An unknown token is indistinguishable from an expired one.
    assert!(sessions::lookup(&pool, &token(99), NOW).await.unwrap().is_none());
}

#[sqlx::test]
async fn touch_slides_the_window_forward(pool: SqlitePool) {
    sessions::create(&pool, &token(1), NOW, NOW + HOUR, None, None)
        .await
        .unwrap();

    let later = NOW + 30 * 60 * 1000;
    sessions::touch(&pool, &token(1), later, later + 24 * HOUR)
        .await
        .unwrap();

    let found = sessions::lookup(&pool, &token(1), later).await.unwrap().unwrap();
    assert_eq!(found.last_seen_at, later);
    assert_eq!(found.expires_at, later + 24 * HOUR);
    assert_eq!(found.created_at, NOW, "created_at must not move");

    // The session now outlives its original expiry — that is the whole point.
    assert!(sessions::lookup(&pool, &token(1), NOW + 2 * HOUR).await.unwrap().is_some());
}

#[sqlx::test]
async fn touch_affects_only_the_named_session(pool: SqlitePool) {
    sessions::create(&pool, &token(1), NOW, NOW + HOUR, None, None).await.unwrap();
    sessions::create(&pool, &token(2), NOW, NOW + HOUR, None, None).await.unwrap();

    sessions::touch(&pool, &token(1), NOW + 1, NOW + 100 * HOUR).await.unwrap();

    let other = sessions::lookup(&pool, &token(2), NOW).await.unwrap().unwrap();
    assert_eq!(other.expires_at, NOW + HOUR, "a missing WHERE clause would move this");
}

#[sqlx::test]
async fn delete_removes_one_session_and_delete_all_removes_the_rest(pool: SqlitePool) {
    for seed in 1..=3 {
        sessions::create(&pool, &token(seed), NOW, NOW + HOUR, None, None).await.unwrap();
    }

    sessions::delete(&pool, &token(2)).await.unwrap();
    assert!(sessions::lookup(&pool, &token(2), NOW).await.unwrap().is_none());
    assert_eq!(sessions::count_active(&pool, NOW).await.unwrap(), 2);

    assert_eq!(sessions::delete_all(&pool).await.unwrap(), 2);
    assert_eq!(sessions::count_active(&pool, NOW).await.unwrap(), 0);
}

#[sqlx::test]
async fn purge_expired_removes_only_dead_rows(pool: SqlitePool) {
    sessions::create(&pool, &token(1), NOW, NOW + HOUR, None, None).await.unwrap();
    sessions::create(&pool, &token(2), NOW, NOW - HOUR, None, None).await.unwrap();
    sessions::create(&pool, &token(3), NOW, NOW - 1, None, None).await.unwrap();

    assert_eq!(sessions::purge_expired(&pool, NOW).await.unwrap(), 2);

    let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(remaining, 1, "the live session must survive");
    assert!(sessions::lookup(&pool, &token(1), NOW).await.unwrap().is_some());
}

#[sqlx::test]
async fn count_active_ignores_expired_rows(pool: SqlitePool) {
    sessions::create(&pool, &token(1), NOW, NOW + HOUR, None, None).await.unwrap();
    sessions::create(&pool, &token(2), NOW, NOW - HOUR, None, None).await.unwrap();

    assert_eq!(sessions::count_active(&pool, NOW).await.unwrap(), 1);
    assert_eq!(sessions::count_active(&pool, NOW + 2 * HOUR).await.unwrap(), 0);
}
