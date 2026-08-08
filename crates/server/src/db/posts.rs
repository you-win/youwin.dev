//! Every statement that touches `posts`.
//!
//! All SQL lives here rather than in handlers. Queries are runtime-checked (see
//! DESIGN.md "Queries"), so the compiler cannot catch a renamed column — which
//! makes "a query you can't find is a query no test covers" a real risk. Keeping
//! them in one file is what makes the test-per-statement rule enforceable.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::Rng as _;
use sqlx::SqlitePool;

use crate::render::markdown;

#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(rename_all = "lowercase")]
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
