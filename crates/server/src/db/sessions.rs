//! Every statement that touches `sessions`.
//!
//! Rows are keyed by SHA-256 of the cookie value, never the value itself, so
//! this table is useless to anyone who reads it.

use sqlx::SqlitePool;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Session {
    pub created_at: i64,
    pub expires_at: i64,
    pub last_seen_at: i64,
}

pub async fn create(
    pool: &SqlitePool,
    token_hash: &[u8],
    now: i64,
    expires_at: i64,
    user_agent: Option<&str>,
    ip: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO sessions (token_hash, created_at, expires_at, last_seen_at, user_agent, ip)
         VALUES (?1, ?2, ?3, ?2, ?4, ?5)",
    )
    .bind(token_hash)
    .bind(now)
    .bind(expires_at)
    .bind(user_agent)
    .bind(ip)
    .execute(pool)
    .await?;

    Ok(())
}

/// Looks up a live session. Expiry is enforced in SQL rather than in Rust so
/// there is no path where a caller forgets to check it.
pub async fn lookup(
    pool: &SqlitePool,
    token_hash: &[u8],
    now: i64,
) -> Result<Option<Session>, sqlx::Error> {
    sqlx::query_as(
        "SELECT created_at, expires_at, last_seen_at
           FROM sessions
          WHERE token_hash = ?1 AND expires_at > ?2",
    )
    .bind(token_hash)
    .bind(now)
    .fetch_optional(pool)
    .await
}

/// Slides the window forward. Called only when the session is more than a day
/// stale, so an active client is not a write on every request.
pub async fn touch(
    pool: &SqlitePool,
    token_hash: &[u8],
    now: i64,
    expires_at: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE sessions SET last_seen_at = ?2, expires_at = ?3 WHERE token_hash = ?1")
        .bind(token_hash)
        .bind(now)
        .bind(expires_at)
        .execute(pool)
        .await?;

    Ok(())
}

pub async fn delete(pool: &SqlitePool, token_hash: &[u8]) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = ?1")
        .bind(token_hash)
        .execute(pool)
        .await?;

    Ok(())
}

/// "Log out everywhere" — the lever to pull when a device goes missing.
pub async fn delete_all(pool: &SqlitePool) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query("DELETE FROM sessions")
        .execute(pool)
        .await?
        .rows_affected())
}

/// Expired rows are already unusable (see `lookup`); this just stops the table
/// growing forever. Runs once at startup.
pub async fn purge_expired(pool: &SqlitePool, now: i64) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query("DELETE FROM sessions WHERE expires_at <= ?1")
        .bind(now)
        .execute(pool)
        .await?
        .rows_affected())
}

/// Live session count, for `/api/auth/me`.
pub async fn count_active(pool: &SqlitePool, now: i64) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM sessions WHERE expires_at > ?1")
        .bind(now)
        .fetch_one(pool)
        .await
}
