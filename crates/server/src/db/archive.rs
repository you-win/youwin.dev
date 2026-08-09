//! Every statement that reads `posts` by date.
//!
//! Separate from `db::posts` for the same reason `db::tags` and `db::search`
//! are: those hold the statements a *surface* needs rather than the ones the
//! table needs, and the test-per-statement rule is easier to keep when the
//! surface and its queries are in one file.
//!
//! Two shapes here, and the difference between them is the whole design:
//!
//! - A **month** is a contiguous range of `created_at`, so it is a keyset scan
//!   over `idx_posts_feed` with the bounds computed in Rust ([`crate::calendar`]).
//! - A **day of the year** is not contiguous — it is one day out of every year —
//!   so it has to compare a formatted date, which no index can help with.
//!
//! The second is a full scan of the feed index, deliberately. On an archive of a
//! few thousand posts it is sub-millisecond, it is behind the same five-minute
//! edge cache as everything else, and the alternative is a generated column plus
//! an index to make one page fast that nobody visits in a loop.

use sqlx::SqlitePool;

use crate::{
    db::posts::{Cursor, FeedRow},
    mood::Mood,
};

/// One month, and how many public posts fall in it.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MonthCount {
    /// `YYYY-MM`. Parsed back into a [`crate::calendar::YearMonth`] by the view.
    pub month: String,
    pub posts: i64,
}

/// One month's worth of one mood — or of having picked none.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MoodCount {
    pub month: String,
    /// `None` is "did not say", which is not `Some(Neutral)`. Keeping them apart
    /// all the way to the wire is the whole reason this query exists: the
    /// distinction is what the composer records and the familiar acts on, and a
    /// chart that folded them together would show a habit that isn't there.
    pub mood: Option<Mood>,
    pub posts: i64,
}

// Replies count. A reply is something that was sat down and written on a day,
// and a spine that hid them would disagree with the month pages below — which
// list them, exactly as the tag pages do.
const MONTHS: &str = "
    SELECT strftime('%Y-%m', created_at / 1000, 'unixepoch') AS month,
           count(*) AS posts
      FROM posts
     WHERE deleted_at IS NULL AND visibility = 'public'
     GROUP BY month
     ORDER BY month DESC";

const MONTH_PAGE: &str = "
    SELECT p.id, p.public_id, p.parent_id, p.root_id,
           p.body_html, p.body_text, p.visibility, p.mood, p.created_at, p.edited_at,
           (SELECT count(*) FROM posts r
             WHERE r.root_id = p.id AND r.id <> p.id
               AND r.deleted_at IS NULL AND r.visibility = 'public') AS reply_count
      FROM posts p
     WHERE p.deleted_at IS NULL
       AND p.visibility = 'public'
       AND p.created_at >= ?1 AND p.created_at < ?2
       AND (p.created_at, p.id) < (?3, ?4)
     ORDER BY p.created_at DESC, p.id DESC
     LIMIT ?5";

const DAY_PAGE: &str = "
    SELECT p.id, p.public_id, p.parent_id, p.root_id,
           p.body_html, p.body_text, p.visibility, p.mood, p.created_at, p.edited_at,
           (SELECT count(*) FROM posts r
             WHERE r.root_id = p.id AND r.id <> p.id
               AND r.deleted_at IS NULL AND r.visibility = 'public') AS reply_count
      FROM posts p
     WHERE p.deleted_at IS NULL
       AND p.visibility = 'public'
       AND strftime('%m-%d', p.created_at / 1000, 'unixepoch') = ?1
       AND (p.created_at, p.id) < (?2, ?3)
     ORDER BY p.created_at DESC, p.id DESC
     LIMIT ?4";

// Every visibility, drafts included — this one answers a question the author is
// asking about their own writing, and a draft was still written in a mood. It is
// reachable only from the authoring host, which is what makes that safe.
const MOODS_BY_MONTH: &str = "
    SELECT strftime('%Y-%m', created_at / 1000, 'unixepoch') AS month,
           mood,
           count(*) AS posts
      FROM posts
     WHERE deleted_at IS NULL
     GROUP BY month, mood
     ORDER BY month DESC";

/// Every month with at least one public post, newest first.
///
/// Zero-padding in the `strftime` format is what makes `ORDER BY month DESC` a
/// chronological sort rather than one that files October before September.
pub async fn months(pool: &SqlitePool) -> Result<Vec<MonthCount>, sqlx::Error> {
    sqlx::query_as(MONTHS).fetch_all(pool).await
}

/// How each month was written, by mood, newest month first.
///
/// One row per (month, mood) pair that occurred, so a mood never used is absent
/// rather than zero — the caller fills the gaps, because it is the one that
/// knows the full set.
pub async fn moods_by_month(pool: &SqlitePool) -> Result<Vec<MoodCount>, sqlx::Error> {
    sqlx::query_as(MOODS_BY_MONTH).fetch_all(pool).await
}

/// One page of a calendar month, newest first.
///
/// `start` and `end` are the half-open millisecond bounds from
/// [`crate::calendar::YearMonth::bounds`].
pub async fn month_page(
    pool: &SqlitePool,
    start: i64,
    end: i64,
    cursor: Cursor,
    limit: i64,
) -> Result<(Vec<FeedRow>, Option<Cursor>), sqlx::Error> {
    let rows: Vec<FeedRow> = sqlx::query_as(MONTH_PAGE)
        .bind(start)
        .bind(end)
        .bind(cursor.created_at)
        .bind(cursor.id)
        .bind(limit + 1)
        .fetch_all(pool)
        .await?;

    Ok(paginate(rows, limit))
}

/// One page of a day of the year, across every year, newest first.
///
/// `key` is `MM-DD` — see [`crate::calendar::MonthDay::key`].
pub async fn day_page(
    pool: &SqlitePool,
    key: &str,
    cursor: Cursor,
    limit: i64,
) -> Result<(Vec<FeedRow>, Option<Cursor>), sqlx::Error> {
    let rows: Vec<FeedRow> = sqlx::query_as(DAY_PAGE)
        .bind(key)
        .bind(cursor.created_at)
        .bind(cursor.id)
        .bind(limit + 1)
        .fetch_all(pool)
        .await?;

    Ok(paginate(rows, limit))
}

/// Turns the `limit + 1` rows every query above fetches into a page plus the
/// cursor for the next one.
///
/// The extra row is how "there is more" is distinguished from "this page is
/// exactly full" without a second `count(*)`.
fn paginate(mut rows: Vec<FeedRow>, limit: i64) -> (Vec<FeedRow>, Option<Cursor>) {
    let next = if rows.len() as i64 > limit {
        rows.truncate(limit as usize);
        rows.last().map(|row| Cursor {
            created_at: row.post.created_at,
            id: row.post.id,
        })
    } else {
        None
    };

    (rows, next)
}
