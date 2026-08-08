//! Every statement that touches `tags` and `post_tags`.
//!
//! Extraction lives in `render::markdown` — it is the same pass that writes the
//! links — so this module only ever stores what it is handed.

use sqlx::{SqliteConnection, SqlitePool};

use crate::{
    db::posts::{Cursor, FeedRow},
    tag,
};

/// A tag with how many public posts carry it.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TagCount {
    /// Canonical (lowercased) form — what a URL carries.
    pub tag: String,
    /// The casing it was first written in — what a page shows.
    pub display: String,
    pub posts: i64,
}

/// Replaces a post's tags.
///
/// Delete-then-insert rather than a diff: a post has a handful of tags, the
/// whole thing runs inside the caller's transaction, and a diff would be more
/// code than the operation it optimizes.
///
/// Takes a connection rather than a pool because it must join the transaction
/// that wrote the post. Tags landing in a separate transaction would be visible
/// on a tag page before the post they belong to existed.
pub async fn sync(
    conn: &mut SqliteConnection,
    post_id: i64,
    tags: &[String],
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM post_tags WHERE post_id = ?1")
        .bind(post_id)
        .execute(&mut *conn)
        .await?;

    for name in tags {
        let canonical = tag::canonical(name);

        // Two statements rather than `ON CONFLICT … RETURNING id`, which needs a
        // no-op `DO UPDATE` to produce a row at all. `DO NOTHING` also leaves the
        // first-written casing in `display` alone, which is the intent.
        sqlx::query("INSERT INTO tags (tag, display) VALUES (?1, ?2) ON CONFLICT(tag) DO NOTHING")
            .bind(&canonical)
            .bind(name)
            .execute(&mut *conn)
            .await?;

        let tag_id: i64 = sqlx::query_scalar("SELECT id FROM tags WHERE tag = ?1")
            .bind(&canonical)
            .fetch_one(&mut *conn)
            .await?;

        sqlx::query("INSERT OR IGNORE INTO post_tags (post_id, tag_id) VALUES (?1, ?2)")
            .bind(post_id)
            .bind(tag_id)
            .execute(&mut *conn)
            .await?;
    }

    Ok(())
}

// Every post carrying a tag, not only thread roots: a hashtag on a reply is
// still a hashtag, and a tag page that hid it would be lying about what it
// lists. Each row links to its own permalink, which renders the whole thread.
const TAG_FEED: &str = "
    SELECT p.id, p.public_id, p.parent_id, p.root_id,
           p.body_html, p.body_text, p.visibility, p.created_at, p.edited_at,
           (SELECT count(*) FROM posts r
             WHERE r.root_id = p.id AND r.id <> p.id
               AND r.deleted_at IS NULL AND r.visibility = 'public') AS reply_count
      FROM post_tags pt
      JOIN tags  t ON t.id = pt.tag_id
      JOIN posts p ON p.id = pt.post_id
     WHERE t.tag = ?1
       AND p.deleted_at IS NULL
       AND p.visibility = 'public'
       AND (p.created_at, p.id) < (?2, ?3)
     ORDER BY p.created_at DESC, p.id DESC
     LIMIT ?4";

// The inner joins do the filtering: a tag whose every post was deleted or
// unpublished has no rows here and simply does not appear. That also means
// `tags` can hold rows nothing references — harmless, and cheaper than
// garbage-collecting on every edit.
const ALL_TAGS: &str = "
    SELECT t.tag, t.display, count(*) AS posts
      FROM tags t
      JOIN post_tags pt ON pt.tag_id = t.id
      JOIN posts     p  ON p.id = pt.post_id
     WHERE p.deleted_at IS NULL AND p.visibility = 'public'
     GROUP BY t.id
     ORDER BY posts DESC, t.tag ASC";

/// One page of posts carrying `tag`, newest first. `tag` is canonicalized here,
/// so a caller may pass whatever casing arrived in the URL.
pub async fn feed_page(
    pool: &SqlitePool,
    tag_name: &str,
    cursor: Cursor,
    limit: i64,
) -> Result<(Vec<FeedRow>, Option<Cursor>), sqlx::Error> {
    let mut rows: Vec<FeedRow> = sqlx::query_as(TAG_FEED)
        .bind(tag::canonical(tag_name))
        .bind(cursor.created_at)
        .bind(cursor.id)
        .bind(limit + 1)
        .fetch_all(pool)
        .await?;

    let next = if rows.len() as i64 > limit {
        rows.truncate(limit as usize);
        rows.last().map(|row| Cursor {
            created_at: row.post.created_at,
            id: row.post.id,
        })
    } else {
        None
    };

    Ok((rows, next))
}

/// The casing a tag was first written in, or `None` if nothing uses it.
///
/// Distinct from "the tag page is empty": a tag can exist in `tags` with every
/// post that used it deleted, and that page should read as empty rather than 404
/// — the URL was valid once and may be again.
pub async fn display_name(pool: &SqlitePool, tag_name: &str) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT display FROM tags WHERE tag = ?1")
        .bind(tag::canonical(tag_name))
        .fetch_optional(pool)
        .await
}

/// Every tag with at least one public post, most-used first.
pub async fn all(pool: &SqlitePool) -> Result<Vec<TagCount>, sqlx::Error> {
    sqlx::query_as(ALL_TAGS).fetch_all(pool).await
}

/// `(post rowid, display name)` for every tagged post, for `export` to group.
///
/// One query rather than one per post: an export walks the whole table anyway,
/// and this keeps it two round trips instead of one per row.
pub async fn by_post(pool: &SqlitePool) -> Result<Vec<(i64, String)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT pt.post_id, t.display
           FROM post_tags pt
           JOIN tags t ON t.id = pt.tag_id
          ORDER BY pt.post_id, t.tag",
    )
    .fetch_all(pool)
    .await
}
