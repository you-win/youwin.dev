//! The one statement the familiar needs.
//!
//! Separate from `db::posts` because it selects a different shape for a
//! different reason: not a page of things to read, but the whole archive
//! reduced to two columns.

use sqlx::SqlitePool;

use crate::familiar::Morsel;

// Public posts only, replies included.
//
// **Replies count.** A reply is something the author sat down and wrote, so
// leaving them out would tell the pet that a long thread was one post and give a
// misleading picture of both cadence and volume.
//
// **Unlisted and drafts do not.** Not a visibility technicality — the counter is
// rendered on a public page, so including them would let a visitor infer that
// unlisted posts exist and roughly when they were written, which is the one
// thing `unlisted` is for. The familiar feeds on the public archive, and it can
// only say what that archive already says.
//
// No LIMIT. This reads every public body on each cache miss, which for a
// microblog of a few thousand short posts is a scan of a megabyte or so out of
// SQLite's page cache, at most once every five minutes — and the diet and mood
// splits on `/familiar` are over the whole archive, so a window would have to be
// two queries to buy nothing.
const ALL: &str = "
    SELECT created_at, body_text, mood
      FROM posts
     WHERE deleted_at IS NULL AND visibility = 'public'
     ORDER BY created_at ASC, id ASC";

/// Every public post, oldest first, as the familiar reads them.
///
/// The order is load-bearing: [`crate::familiar::compute`] and
/// [`crate::familiar::stats`] both binary-search this slice by timestamp.
pub async fn all(pool: &SqlitePool) -> Result<Vec<Morsel>, sqlx::Error> {
    sqlx::query_as(ALL).fetch_all(pool).await
}
