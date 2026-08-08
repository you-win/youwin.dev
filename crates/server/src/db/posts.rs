//! Every statement that touches `posts`.
//!
//! All SQL lives here rather than in handlers. Queries are runtime-checked (see
//! DESIGN.md "Queries"), so the compiler cannot catch a renamed column — which
//! makes "a query you can't find is a query no test covers" a real risk. Keeping
//! them in one file is what makes the test-per-statement rule enforceable.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::Rng as _;
use sqlx::SqlitePool;

use crate::{db::tags, render::markdown};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, sqlx::Type, serde::Serialize, serde::Deserialize,
)]
#[sqlx(rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    /// In the feed, in the Atom document, indexable.
    Public,
    /// Reachable at its permalink, but listed nowhere and never indexed.
    Unlisted,
    /// A 404 on the public site — indistinguishable from a bad id.
    Draft,
}

/// A post as the public site needs it.
///
/// Deliberately without `body` (the markdown source): the public surface never
/// re-renders, so shipping the source to it would be dead weight. M3's edit form
/// fetches it from the authoring API instead.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Post {
    pub id: i64,
    pub public_id: String,
    pub parent_id: Option<i64>,
    pub root_id: i64,
    pub body_html: String,
    pub body_text: String,
    pub visibility: Visibility,
    pub created_at: i64,
    pub edited_at: Option<i64>,
}

/// A feed row: a thread root plus how many replies hang off it.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct FeedRow {
    #[sqlx(flatten)]
    pub post: Post,
    pub reply_count: i64,
}

/// A post as the *authoring* side needs it: everything above plus `body`, the
/// markdown source an edit form loads. Only this side pays for that column.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AuthoredRow {
    #[sqlx(flatten)]
    pub post: Post,
    pub body: String,
    pub reply_count: i64,
}

impl Visibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Unlisted => "unlisted",
            Self::Draft => "draft",
        }
    }

    /// A draft is unpublished, so editing one is not "editing" in the sense the
    /// `edited` marker means. Only published posts grow an `edited_at`.
    pub fn is_published(self) -> bool {
        matches!(self, Self::Public | Self::Unlisted)
    }
}

/// Keyset pagination position. Opaque to the client; `{created_at}:{id}` inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub created_at: i64,
    pub id: i64,
}

impl Cursor {
    /// The first page.
    ///
    /// Every real row sorts strictly before this, so one query shape serves both
    /// the first page and every page after it — and the keyset predicate stays
    /// indexed in both cases. The alternative, `(?1 IS NULL OR …)`, would defeat
    /// the index on the most-requested page on the site.
    pub const START: Self = Self {
        created_at: i64::MAX,
        id: i64::MAX,
    };

    pub fn encode(self) -> String {
        URL_SAFE_NO_PAD.encode(format!("{}:{}", self.created_at, self.id))
    }

    pub fn decode(raw: &str) -> Option<Self> {
        let bytes = URL_SAFE_NO_PAD.decode(raw).ok()?;
        let text = String::from_utf8(bytes).ok()?;
        let (created_at, id) = text.split_once(':')?;
        Some(Self {
            created_at: created_at.parse().ok()?,
            id: id.parse().ok()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursors_round_trip() {
        let cursor = Cursor {
            created_at: 1_786_259_199_000,
            id: 42,
        };
        assert_eq!(Cursor::decode(&cursor.encode()), Some(cursor));
        assert_eq!(Cursor::decode(&Cursor::START.encode()), Some(Cursor::START));
    }

    #[test]
    fn malformed_cursors_decode_to_none_rather_than_panicking() {
        // These arrive straight off the query string, so every branch here is
        // reachable by anyone typing into the URL bar.
        for bad in ["", "!!!", "bm90LWEtY3Vyc29y", &URL_SAFE_NO_PAD.encode("1:2:3")] {
            assert_eq!(Cursor::decode(bad), None, "{bad:?} should not decode");
        }
    }

    #[test]
    fn public_ids_are_sixteen_chars_and_url_safe() {
        let id = new_public_id();
        assert_eq!(id.len(), 16);
        assert!(
            id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "{id} must survive a URL path segment unescaped"
        );
        assert_ne!(new_public_id(), new_public_id());
    }
}

/// 12 random bytes, base64url — 16 characters.
///
/// Unguessability is load-bearing for `unlisted`, whose only protection is that
/// nobody can enumerate the id.
pub fn new_public_id() -> String {
    let mut bytes = [0u8; 12];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

// The reply count is a correlated subquery rather than a join + group by: at one
// row per feed page it is cheaper, and it keeps the row shape flat for FromRow.
const FEED_PAGE: &str = "
    SELECT p.id, p.public_id, p.parent_id, p.root_id,
           p.body_html, p.body_text, p.visibility, p.created_at, p.edited_at,
           (SELECT count(*) FROM posts r
             WHERE r.root_id = p.id AND r.id <> p.id
               AND r.deleted_at IS NULL AND r.visibility = 'public') AS reply_count
      FROM posts p
     WHERE p.deleted_at IS NULL
       AND p.parent_id IS NULL
       AND p.visibility = 'public'
       AND (p.created_at, p.id) < (?1, ?2)
     ORDER BY p.created_at DESC, p.id DESC
     LIMIT ?3";

const BY_PUBLIC_ID: &str = "
    SELECT id, public_id, parent_id, root_id,
           body_html, body_text, visibility, created_at, edited_at
      FROM posts
     WHERE public_id = ?1 AND deleted_at IS NULL AND visibility <> 'draft'";

const THREAD: &str = "
    SELECT id, public_id, parent_id, root_id,
           body_html, body_text, visibility, created_at, edited_at
      FROM posts
     WHERE root_id = ?1 AND deleted_at IS NULL AND visibility <> 'draft'
     ORDER BY created_at ASC, id ASC";

// ---------------------------------------------------------------------------
// Authoring queries. These see everything: drafts, unlisted, all of it. They are
// reachable only from write.youwin.dev, behind the session guard.
// ---------------------------------------------------------------------------

// The column list is repeated across the four queries below rather than shared
// through `format!`. sqlx 0.9 refuses runtime-built SQL (`SqlSafeStr`), and the
// escape hatch is `AssertSqlSafe` — not worth taking to save nine duplicated
// lines. Literal queries are also greppable, which the assembled ones were not.
const AUTHORED_FEED: &str = "
    SELECT p.id, p.public_id, p.parent_id, p.root_id,
           p.body, p.body_html, p.body_text, p.visibility, p.created_at, p.edited_at,
           (SELECT count(*) FROM posts r
             WHERE r.root_id = p.id AND r.id <> p.id AND r.deleted_at IS NULL) AS reply_count
      FROM posts p
     WHERE p.deleted_at IS NULL
       AND p.parent_id IS NULL
       AND (p.created_at, p.id) < (?1, ?2)
     ORDER BY p.created_at DESC, p.id DESC
     LIMIT ?3";

const AUTHORED_BY_PUBLIC_ID: &str = "
    SELECT p.id, p.public_id, p.parent_id, p.root_id,
           p.body, p.body_html, p.body_text, p.visibility, p.created_at, p.edited_at,
           (SELECT count(*) FROM posts r
             WHERE r.root_id = p.id AND r.id <> p.id AND r.deleted_at IS NULL) AS reply_count
      FROM posts p
     WHERE p.public_id = ?1 AND p.deleted_at IS NULL";

const AUTHORED_THREAD: &str = "
    SELECT p.id, p.public_id, p.parent_id, p.root_id,
           p.body, p.body_html, p.body_text, p.visibility, p.created_at, p.edited_at,
           (SELECT count(*) FROM posts r
             WHERE r.root_id = p.id AND r.id <> p.id AND r.deleted_at IS NULL) AS reply_count
      FROM posts p
     WHERE p.root_id = ?1 AND p.deleted_at IS NULL
     ORDER BY p.created_at ASC, p.id ASC";

const AUTHORED_DRAFTS: &str = "
    SELECT p.id, p.public_id, p.parent_id, p.root_id,
           p.body, p.body_html, p.body_text, p.visibility, p.created_at, p.edited_at,
           (SELECT count(*) FROM posts r
             WHERE r.root_id = p.id AND r.id <> p.id AND r.deleted_at IS NULL) AS reply_count
      FROM posts p
     WHERE p.deleted_at IS NULL AND p.visibility = 'draft'
     ORDER BY p.created_at DESC, p.id DESC";

/// One page of thread roots at every visibility, newest first.
pub async fn authored_feed(
    pool: &SqlitePool,
    cursor: Cursor,
    limit: i64,
) -> Result<(Vec<AuthoredRow>, Option<Cursor>), sqlx::Error> {
    let mut rows: Vec<AuthoredRow> = sqlx::query_as(AUTHORED_FEED)
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

/// A post by public id, drafts included. The authoring counterpart to
/// `by_public_id`, which hides them.
pub async fn authored_by_public_id(
    pool: &SqlitePool,
    public_id: &str,
) -> Result<Option<AuthoredRow>, sqlx::Error> {
    sqlx::query_as(AUTHORED_BY_PUBLIC_ID)
        .bind(public_id)
        .fetch_optional(pool)
        .await
}

/// A whole thread, drafts included, oldest first.
pub async fn authored_thread(
    pool: &SqlitePool,
    root_id: i64,
) -> Result<Vec<AuthoredRow>, sqlx::Error> {
    sqlx::query_as(AUTHORED_THREAD)
        .bind(root_id)
        .fetch_all(pool)
        .await
}

/// Drafts, newest first. Replies included — a half-written reply is still a
/// draft you want to find again.
pub async fn drafts(pool: &SqlitePool) -> Result<Vec<AuthoredRow>, sqlx::Error> {
    sqlx::query_as(AUTHORED_DRAFTS).fetch_all(pool).await
}

/// Edits a post, re-rendering when the body changed.
///
/// `edited_at` is set only when a *published* post's body actually changes:
/// re-saving a draft is not an edit in the sense the marker means, and neither
/// is flipping visibility without touching the text.
pub async fn update(
    pool: &SqlitePool,
    public_id: &str,
    body: Option<&str>,
    visibility: Option<Visibility>,
    now: i64,
) -> Result<Option<AuthoredRow>, sqlx::Error> {
    let mut tx = pool.begin().await?;

    let current: Option<(i64, String, Visibility, Option<i64>)> = sqlx::query_as(
        "SELECT id, body, visibility, edited_at FROM posts
          WHERE public_id = ?1 AND deleted_at IS NULL",
    )
    .bind(public_id)
    .fetch_optional(&mut *tx)
    .await?;

    let Some((id, current_body, current_visibility, current_edited)) = current else {
        return Ok(None);
    };

    let new_body = body.unwrap_or(&current_body);
    let body_changed = new_body != current_body;
    let rendered = markdown::render(new_body);
    let new_visibility = visibility.unwrap_or(current_visibility);

    let edited_at = if body_changed && current_visibility.is_published() {
        Some(now)
    } else {
        current_edited
    };

    sqlx::query(
        "UPDATE posts
            SET body = ?2, body_html = ?3, body_text = ?4,
                visibility = ?5, updated_at = ?6, edited_at = ?7
          WHERE id = ?1",
    )
    .bind(id)
    .bind(new_body)
    .bind(&rendered.html)
    .bind(&rendered.text)
    .bind(new_visibility)
    .bind(now)
    .bind(edited_at)
    .execute(&mut *tx)
    .await?;

    // Unconditional, not gated on `body_changed`: the tag pass is part of
    // rendering, so re-running it is how a change to the extraction rules takes
    // effect on the next edit. It is also cheap — a handful of rows.
    tags::sync(&mut tx, id, &rendered.tags).await?;

    tx.commit().await?;

    authored_by_public_id(pool, public_id).await
}

/// Soft-deletes a post, and a thread when the post is its root.
///
/// Deleting a root must take its replies with it, or they stay reachable at
/// their own permalinks with the post they answered gone. A reply deletes alone.
/// Returns the number of rows affected, or `None` if there was nothing to delete.
pub async fn soft_delete(
    pool: &SqlitePool,
    public_id: &str,
    now: i64,
) -> Result<Option<u64>, sqlx::Error> {
    let mut tx = pool.begin().await?;

    let target: Option<(i64, Option<i64>)> = sqlx::query_as(
        "SELECT id, parent_id FROM posts WHERE public_id = ?1 AND deleted_at IS NULL",
    )
    .bind(public_id)
    .fetch_optional(&mut *tx)
    .await?;

    let Some((id, parent_id)) = target else {
        return Ok(None);
    };

    let affected = if parent_id.is_none() {
        sqlx::query(
            "UPDATE posts SET deleted_at = ?2, updated_at = ?2
              WHERE root_id = ?1 AND deleted_at IS NULL",
        )
        .bind(id)
        .bind(now)
        .execute(&mut *tx)
        .await?
        .rows_affected()
    } else {
        sqlx::query("UPDATE posts SET deleted_at = ?2, updated_at = ?2 WHERE id = ?1")
            .bind(id)
            .bind(now)
            .execute(&mut *tx)
            .await?
            .rows_affected()
    };

    tx.commit().await?;
    Ok(Some(affected))
}

/// A post as `export` dumps it: every column, plus the parent and thread-root
/// ids translated out of rowids so the archive stands on its own.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ExportRow {
    #[serde(skip)]
    pub id: i64,
    pub public_id: String,
    pub parent_public_id: Option<String>,
    pub root_public_id: String,
    pub body: String,
    pub body_html: String,
    pub body_text: String,
    pub visibility: Visibility,
    pub created_at: i64,
    pub updated_at: i64,
    pub edited_at: Option<i64>,
    pub deleted_at: Option<i64>,
}

// Deleted posts included. This is the backup, and a backup that silently drops
// what you deleted is not one — soft deletion is recoverable precisely because
// the row survives, and the export should preserve that property.
const EXPORT_ALL: &str = "
    SELECT p.id, p.public_id,
           parent.public_id AS parent_public_id,
           root.public_id   AS root_public_id,
           p.body, p.body_html, p.body_text, p.visibility,
           p.created_at, p.updated_at, p.edited_at, p.deleted_at
      FROM posts p
      LEFT JOIN posts parent ON parent.id = p.parent_id
      JOIN      posts root   ON root.id = p.root_id
     ORDER BY p.created_at ASC, p.id ASC";

/// Every post ever written, oldest first.
pub async fn export_all(pool: &SqlitePool) -> Result<Vec<ExportRow>, sqlx::Error> {
    sqlx::query_as(EXPORT_ALL).fetch_all(pool).await
}

/// What a re-render changed.
#[derive(Debug, Default, Clone, Copy)]
pub struct Rerendered {
    pub scanned: u64,
    pub rewritten: u64,
}

/// Rebuilds `body_html`, `body_text` and the tag rows from `body`.
///
/// `body` is the authority; the other three are a cache of what the renderer
/// made of it. Change the renderer — add hashtag linking, say — and every post
/// written before the change is stale until this runs.
///
/// Deliberately does not touch `updated_at` or `edited_at`. Re-rendering is not
/// an edit: the post says what it always said, and marking a decade of archive
/// as "edited" because a sanitizer rule changed would be a lie in the one place
/// the site makes a promise about its own history.
///
/// One transaction per post rather than one for all of them, so an interrupted
/// run leaves a consistent database that a second run simply finishes.
pub async fn rerender_all(pool: &SqlitePool) -> Result<Rerendered, sqlx::Error> {
    let rows: Vec<(i64, String, String, String)> =
        sqlx::query_as("SELECT id, body, body_html, body_text FROM posts ORDER BY id")
            .fetch_all(pool)
            .await?;

    let mut result = Rerendered::default();

    for (id, body, old_html, old_text) in rows {
        result.scanned += 1;
        let rendered = markdown::render(&body);
        let mut tx = pool.begin().await?;

        if rendered.html != old_html || rendered.text != old_text {
            sqlx::query("UPDATE posts SET body_html = ?2, body_text = ?3 WHERE id = ?1")
                .bind(id)
                .bind(&rendered.html)
                .bind(&rendered.text)
                .execute(&mut *tx)
                .await?;
            result.rewritten += 1;
        }

        // Unconditional: tags can need rebuilding even when the HTML is byte
        // identical, because the first run after 0002 starts from no tag rows at
        // all while the HTML for an untagged post is unchanged.
        tags::sync(&mut tx, id, &rendered.tags).await?;
        tx.commit().await?;
    }

    Ok(result)
}

/// Resolves a public id to a rowid, for turning a `parent_public_id` from the
/// wire into the `parent_id` `insert` wants.
pub async fn id_for_public_id(
    pool: &SqlitePool,
    public_id: &str,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM posts WHERE public_id = ?1 AND deleted_at IS NULL")
        .bind(public_id)
        .fetch_optional(pool)
        .await
}

/// One page of thread roots, newest first.
///
/// Fetches `limit + 1` so the caller can tell "there is another page" from "this
/// page happens to be full" without a second count query.
pub async fn feed_page(
    pool: &SqlitePool,
    cursor: Cursor,
    limit: i64,
) -> Result<(Vec<FeedRow>, Option<Cursor>), sqlx::Error> {
    let mut rows: Vec<FeedRow> = sqlx::query_as(FEED_PAGE)
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

/// A single post by its public id. `None` for drafts, deletions, and bad ids
/// alike — the caller cannot tell them apart, and neither can a visitor.
pub async fn by_public_id(pool: &SqlitePool, public_id: &str) -> Result<Option<Post>, sqlx::Error> {
    sqlx::query_as(BY_PUBLIC_ID)
        .bind(public_id)
        .fetch_optional(pool)
        .await
}

/// Every visible post in a thread, oldest first. One indexed range scan, which
/// is what `root_id` is denormalized for.
pub async fn thread(pool: &SqlitePool, root_id: i64) -> Result<Vec<Post>, sqlx::Error> {
    sqlx::query_as(THREAD).bind(root_id).fetch_all(pool).await
}

/// Writes a post, rendering the markdown on the way in.
///
/// `root_id` is NOT NULL but a new thread root cannot know its own id until the
/// insert returns, so the row is written with a placeholder and corrected inside
/// the same transaction. A reply inherits its parent's `root_id` and needs no
/// second statement.
pub async fn insert(
    pool: &SqlitePool,
    body: &str,
    parent_id: Option<i64>,
    visibility: Visibility,
    created_at: i64,
) -> Result<Post, sqlx::Error> {
    let rendered = markdown::render(body);
    let public_id = new_public_id();

    let mut tx = pool.begin().await?;

    let parent_root: Option<i64> = match parent_id {
        Some(parent) => Some(
            sqlx::query_scalar("SELECT root_id FROM posts WHERE id = ?1 AND deleted_at IS NULL")
                .bind(parent)
                .fetch_one(&mut *tx)
                .await?,
        ),
        None => None,
    };

    let id: i64 = sqlx::query_scalar(
        "INSERT INTO posts
            (public_id, parent_id, root_id, body, body_html, body_text,
             visibility, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
         RETURNING id",
    )
    .bind(&public_id)
    .bind(parent_id)
    .bind(parent_root.unwrap_or(0))
    .bind(body)
    .bind(&rendered.html)
    .bind(&rendered.text)
    .bind(visibility)
    .bind(created_at)
    .fetch_one(&mut *tx)
    .await?;

    if parent_root.is_none() {
        sqlx::query("UPDATE posts SET root_id = id WHERE id = ?1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }

    // Inside the same transaction, so a tag page can never list a post that is
    // not yet visible at its permalink.
    tags::sync(&mut tx, id, &rendered.tags).await?;

    tx.commit().await?;

    Ok(Post {
        id,
        public_id,
        parent_id,
        root_id: parent_root.unwrap_or(id),
        body_html: rendered.html,
        body_text: rendered.text,
        visibility,
        created_at,
        edited_at: None,
    })
}
