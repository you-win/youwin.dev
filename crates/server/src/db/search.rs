//! Full-text search over `posts_fts`.
//!
//! Two things here are load-bearing and neither is the SQL. The first is
//! [`fts_query`], which turns whatever someone typed into a query FTS5 will
//! accept — an unescaped `"` reaching MATCH is a 500 on a public page. The
//! second is the snippet markers: FTS5 wraps matched terms in strings we choose,
//! and choosing anything HTML-shaped would mean interpolating database output
//! into a page unescaped, which is the one thing `body_html` earns and nothing
//! else does.

use sqlx::SqlitePool;

use crate::db::posts::{AuthoredRow, Cursor, Post};

/// A public search result: the post, plus the matched fragment of its text.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Hit {
    #[sqlx(flatten)]
    pub post: Post,
    pub reply_count: i64,
    /// Plaintext with matched terms wrapped in [`MARK_OPEN`]/[`MARK_CLOSE`].
    /// Read it with [`segments`] rather than by hand.
    pub snippet: String,
}

/// Delimiters FTS5 wraps matched terms in.
///
/// ASCII STX and ETX: control characters with no meaning in prose, chosen so the
/// snippet is inert text all the way to the template, where [`segments`] splits
/// it and maud escapes each piece. If a body somehow contains one, the worst
/// case is a stray highlight — never markup.
pub const MARK_OPEN: char = '\u{2}';
pub const MARK_CLOSE: char = '\u{3}';

/// Splits a snippet into `(highlighted, text)` runs.
pub fn segments(snippet: &str) -> Vec<(bool, &str)> {
    let mut out = Vec::new();
    let mut rest = snippet;

    while let Some((before, tail)) = rest.split_once(MARK_OPEN) {
        if !before.is_empty() {
            out.push((false, before));
        }
        // An unpaired opener means the snippet was truncated mid-term; treat the
        // remainder as highlighted rather than dropping it.
        match tail.split_once(MARK_CLOSE) {
            Some((marked, next)) => {
                if !marked.is_empty() {
                    out.push((true, marked));
                }
                rest = next;
            }
            None => {
                out.push((true, tail));
                return out;
            }
        }
    }

    if !rest.is_empty() {
        out.push((false, rest));
    }

    out
}

/// Most tokens taken from one query. Past this it is not a search.
const MAX_TOKENS: usize = 16;

/// Turns typed input into an FTS5 MATCH expression, or `None` if there is
/// nothing to search for.
///
/// Every token is quoted, which makes it a literal phrase and takes FTS5's
/// operators — `AND`, `OR`, `NOT`, `NEAR`, `*`, `^`, `:` — off the table along
/// with every way to write a syntax error. Tokens are joined by implicit AND, so
/// more words narrow the result, which is what a search box is expected to do.
///
/// The trade is deliberate: no boolean operators, in exchange for a search box
/// where no input is a 500. A stray apostrophe is far likelier here than a
/// hand-written `NEAR(a b, 3)`.
pub fn fts_query(raw: &str) -> Option<String> {
    let mut out = String::new();

    for token in raw
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .take(MAX_TOKENS)
    {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push('"');
        out.push_str(token);
        out.push('"');
    }

    (!out.is_empty()).then_some(out)
}

// `snippet()` reads the text back out of `posts` — that is what an external
// content index is for. 12 tokens of context is roughly a line, enough to see
// why a post matched without reprinting it.
//
// Ordering is by recency, not `bm25()`. Searching a personal archive is
// re-finding something you know you wrote, where "when" is the strongest
// remaining clue; relevance scoring across 300-character posts mostly ranks on
// length. It also means search paginates with the same keyset cursor as the
// feed instead of needing a score-based one. `ORDER BY bm25(posts_fts)` is a
// one-line change if that ever stops being true.
const SEARCH_PUBLIC: &str = "
    SELECT p.id, p.public_id, p.parent_id, p.root_id,
           p.body_html, p.body_text, p.visibility, p.created_at, p.edited_at,
           (SELECT count(*) FROM posts r
             WHERE r.root_id = p.id AND r.id <> p.id
               AND r.deleted_at IS NULL AND r.visibility = 'public') AS reply_count,
           snippet(posts_fts, 0, char(2), char(3), '…', 12) AS snippet
      FROM posts_fts
      JOIN posts p ON p.id = posts_fts.rowid
     WHERE posts_fts MATCH ?1
       AND p.deleted_at IS NULL
       AND p.visibility = 'public'
       AND (p.created_at, p.id) < (?2, ?3)
     ORDER BY p.created_at DESC, p.id DESC
     LIMIT ?4";

// The authoring counterpart: drafts and unlisted posts included, and `body` so
// a result can be opened straight into the editor. No snippet — the authoring
// app renders whole posts, exactly as its feed does.
const SEARCH_AUTHORED: &str = "
    SELECT p.id, p.public_id, p.parent_id, p.root_id,
           p.body, p.body_html, p.body_text, p.visibility, p.created_at, p.edited_at,
           (SELECT count(*) FROM posts r
             WHERE r.root_id = p.id AND r.id <> p.id AND r.deleted_at IS NULL) AS reply_count
      FROM posts_fts
      JOIN posts p ON p.id = posts_fts.rowid
     WHERE posts_fts MATCH ?1
       AND p.deleted_at IS NULL
       AND (p.created_at, p.id) < (?2, ?3)
     ORDER BY p.created_at DESC, p.id DESC
     LIMIT ?4";

/// One page of public matches, newest first.
pub async fn public(
    pool: &SqlitePool,
    query: &str,
    cursor: Cursor,
    limit: i64,
) -> Result<(Vec<Hit>, Option<Cursor>), sqlx::Error> {
    let mut rows: Vec<Hit> = sqlx::query_as(SEARCH_PUBLIC)
        .bind(query)
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

/// One page of matches across everything, drafts included.
pub async fn authored(
    pool: &SqlitePool,
    query: &str,
    cursor: Cursor,
    limit: i64,
) -> Result<(Vec<AuthoredRow>, Option<Cursor>), sqlx::Error> {
    let mut rows: Vec<AuthoredRow> = sqlx::query_as(SEARCH_AUTHORED)
        .bind(query)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_input_becomes_a_valid_query_or_nothing() {
        assert_eq!(fts_query("hello world").as_deref(), Some(r#""hello" "world""#));
        // The characters that would otherwise be FTS5 syntax errors or operators.
        assert_eq!(fts_query(r#"a" OR b"#).as_deref(), Some(r#""a" "OR" "b""#));
        assert_eq!(fts_query("NEAR(a b, 3)").as_deref(), Some(r#""NEAR" "a" "b" "3""#));
        assert_eq!(fts_query("rust*").as_deref(), Some(r#""rust""#));
        // A hashtag searches for the word; `#` is not a token character.
        assert_eq!(fts_query("#rust").as_deref(), Some(r#""rust""#));

        // Nothing to search for.
        for empty in ["", "   ", "!!!", "\"\""] {
            assert_eq!(fts_query(empty), None, "{empty:?}");
        }
    }

    #[test]
    fn absurd_queries_are_bounded() {
        let long = (0..100).map(|i| i.to_string()).collect::<Vec<_>>().join(" ");
        let built = fts_query(&long).expect("tokens");
        assert_eq!(built.matches('"').count(), MAX_TOKENS * 2);
    }

    #[test]
    fn snippets_split_into_marked_and_unmarked_runs() {
        let snippet = format!("a {MARK_OPEN}hit{MARK_CLOSE} b");
        assert_eq!(segments(&snippet), vec![(false, "a "), (true, "hit"), (false, " b")]);

        // No markers at all: one plain run.
        assert_eq!(segments("plain"), vec![(false, "plain")]);
        assert_eq!(segments(""), vec![]);

        // Truncated mid-term rather than dropping the tail.
        let unpaired = format!("a {MARK_OPEN}hit");
        assert_eq!(segments(&unpaired), vec![(false, "a "), (true, "hit")]);
    }
}
