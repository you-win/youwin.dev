//! One test per statement in `db::posts`.
//!
//! Queries here are runtime-checked, so nothing but these tests stands between a
//! renamed column and a 500 on the front page. Each asserts row *shape*, not
//! merely that the query returned — a `SELECT` that silently stopped matching
//! would still be `Ok(vec![])`.

use sqlx::SqlitePool;
use youwin_server::{
    db::posts::{self, Cursor, Post, Visibility},
    mood::Mood,
};

const T0: i64 = 1_786_000_000_000;

async fn post_at(pool: &SqlitePool, body: &str, offset_minutes: i64, visibility: Visibility) -> Post {
    posts::insert(pool, body, None, visibility, None, T0 + offset_minutes * 60_000)
        .await
        .expect("insert")
}

async fn reply_at(pool: &SqlitePool, parent: &Post, body: &str, offset_minutes: i64) -> Post {
    posts::insert(
        pool,
        body,
        Some(parent.id),
        Visibility::Public,
        None,
        T0 + offset_minutes * 60_000,
    )
    .await
    .expect("insert reply")
}

async fn soft_delete(pool: &SqlitePool, id: i64) {
    sqlx::query("UPDATE posts SET deleted_at = ?1 WHERE id = ?2")
        .bind(T0)
        .bind(id)
        .execute(pool)
        .await
        .expect("soft delete");
}

#[sqlx::test]
async fn insert_renders_markdown_and_populates_both_derived_columns(pool: SqlitePool) {
    let post = post_at(&pool, "hello *world*\nsecond line", 0, Visibility::Public).await;

    assert!(post.body_html.contains("<em>world</em>"), "{}", post.body_html);
    assert!(post.body_html.contains("<br>"), "{}", post.body_html);
    assert_eq!(post.body_text, "hello world\nsecond line");
    assert_eq!(post.public_id.len(), 16, "12 random bytes as base64url");

    // The returned struct must match what actually landed in the row, or the
    // authoring API would report success with different content than it stored.
    let stored = posts::by_public_id(&pool, &post.public_id)
        .await
        .unwrap()
        .expect("stored");
    assert_eq!(stored.body_html, post.body_html);
    assert_eq!(stored.body_text, post.body_text);
    assert_eq!(stored.created_at, post.created_at);
    assert_eq!(stored.visibility, Visibility::Public);
    assert_eq!(stored.edited_at, None);
}

#[sqlx::test]
async fn insert_sets_root_id_to_self_for_roots_and_inherits_it_for_replies(pool: SqlitePool) {
    let root = post_at(&pool, "root", 0, Visibility::Public).await;
    assert_eq!(root.root_id, root.id, "a root is its own thread head");
    assert_eq!(root.parent_id, None);

    let reply = reply_at(&pool, &root, "reply", 5).await;
    assert_eq!(reply.root_id, root.id);
    assert_eq!(reply.parent_id, Some(root.id));

    // A reply to a reply stays on the same thread rather than starting one.
    let nested = posts::insert(&pool, "nested", Some(reply.id), Visibility::Public, None, T0 + 600_000)
        .await
        .unwrap();
    assert_eq!(nested.root_id, root.id);
}

#[sqlx::test]
async fn feed_page_returns_only_public_roots_newest_first(pool: SqlitePool) {
    let first = post_at(&pool, "first", 0, Visibility::Public).await;
    let second = post_at(&pool, "second", 10, Visibility::Public).await;
    reply_at(&pool, &first, "a reply", 20).await;
    post_at(&pool, "unlisted", 30, Visibility::Unlisted).await;
    post_at(&pool, "draft", 40, Visibility::Draft).await;
    let deleted = post_at(&pool, "deleted", 50, Visibility::Public).await;
    soft_delete(&pool, deleted.id).await;

    let (rows, older) = posts::feed_page(&pool, Cursor::START, 20).await.unwrap();

    let ids: Vec<&str> = rows.iter().map(|r| r.post.public_id.as_str()).collect();
    assert_eq!(
        ids,
        vec![second.public_id.as_str(), first.public_id.as_str()],
        "replies, unlisted, drafts, and deleted posts are all excluded, newest first"
    );
    assert!(older.is_none(), "one page holds everything here");

    // Shape, not just presence: an empty body_html would mean FromRow silently
    // matched the wrong column.
    assert!(rows[0].post.body_html.contains("second"));
}

#[sqlx::test]
async fn feed_page_counts_only_visible_replies(pool: SqlitePool) {
    let root = post_at(&pool, "root", 0, Visibility::Public).await;
    reply_at(&pool, &root, "one", 1).await;
    reply_at(&pool, &root, "two", 2).await;

    let hidden = posts::insert(&pool, "draft reply", Some(root.id), Visibility::Draft, None, T0 + 3)
        .await
        .unwrap();
    let removed = reply_at(&pool, &root, "deleted reply", 4).await;
    soft_delete(&pool, removed.id).await;
    let _ = hidden;

    let (rows, _) = posts::feed_page(&pool, Cursor::START, 20).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].reply_count, 2,
        "the root itself, drafts, and deleted replies must not be counted"
    );
}

#[sqlx::test]
async fn feed_page_paginates_by_cursor_without_gaps_or_repeats(pool: SqlitePool) {
    for minute in 0..5 {
        post_at(&pool, &format!("post {minute}"), minute, Visibility::Public).await;
    }

    let (page_one, older) = posts::feed_page(&pool, Cursor::START, 2).await.unwrap();
    assert_eq!(page_one.len(), 2);
    let cursor = older.expect("more pages remain");

    let (page_two, older) = posts::feed_page(&pool, cursor, 2).await.unwrap();
    assert_eq!(page_two.len(), 2);

    let (page_three, older) = posts::feed_page(&pool, older.expect("still more"), 2)
        .await
        .unwrap();
    assert_eq!(page_three.len(), 1, "the tail page is short");
    assert!(older.is_none(), "and reports no further pages");

    let seen: Vec<String> = page_one
        .iter()
        .chain(&page_two)
        .chain(&page_three)
        .map(|r| r.post.public_id.clone())
        .collect();

    let mut unique = seen.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 5, "every post appears exactly once: {seen:?}");

    // Descending by time throughout, across the page boundaries.
    let times: Vec<i64> = page_one
        .iter()
        .chain(&page_two)
        .chain(&page_three)
        .map(|r| r.post.created_at)
        .collect();
    assert!(times.windows(2).all(|w| w[0] > w[1]), "{times:?}");
}

#[sqlx::test]
async fn by_public_id_hides_drafts_and_deletions_indistinguishably(pool: SqlitePool) {
    let public = post_at(&pool, "public", 0, Visibility::Public).await;
    let unlisted = post_at(&pool, "unlisted", 1, Visibility::Unlisted).await;
    let draft = post_at(&pool, "draft", 2, Visibility::Draft).await;
    let deleted = post_at(&pool, "deleted", 3, Visibility::Public).await;
    soft_delete(&pool, deleted.id).await;

    assert!(posts::by_public_id(&pool, &public.public_id).await.unwrap().is_some());
    assert!(
        posts::by_public_id(&pool, &unlisted.public_id).await.unwrap().is_some(),
        "unlisted is reachable by direct link — that is the whole point of it"
    );

    for hidden in [&draft.public_id, &deleted.public_id, &"nope".to_owned()] {
        assert!(
            posts::by_public_id(&pool, hidden).await.unwrap().is_none(),
            "{hidden} must be indistinguishable from a bad id"
        );
    }
}

#[sqlx::test]
async fn thread_returns_the_whole_chain_oldest_first(pool: SqlitePool) {
    let root = post_at(&pool, "root", 0, Visibility::Public).await;
    let first = reply_at(&pool, &root, "first reply", 10).await;
    let second = reply_at(&pool, &root, "second reply", 20).await;

    // Neither of these belongs to the thread.
    let other = post_at(&pool, "unrelated", 30, Visibility::Public).await;
    let hidden = posts::insert(&pool, "draft reply", Some(root.id), Visibility::Draft, None, T0 + 40)
        .await
        .unwrap();

    let chain = posts::thread(&pool, root.root_id).await.unwrap();
    let ids: Vec<i64> = chain.iter().map(|p| p.id).collect();

    assert_eq!(ids, vec![root.id, first.id, second.id]);
    assert!(!ids.contains(&other.id));
    assert!(!ids.contains(&hidden.id), "drafts stay out of public threads");

    // Fetching by a reply's root_id yields the same thread — this is what makes
    // a permalink to a reply render the full conversation.
    let from_reply = posts::thread(&pool, second.root_id).await.unwrap();
    assert_eq!(from_reply.len(), 3);
}

#[sqlx::test]
async fn deleting_a_root_cascades_to_its_replies(pool: SqlitePool) {
    // Not a soft delete: this asserts the ON DELETE CASCADE foreign key is
    // actually live, which it silently would not be if `foreign_keys` regressed.
    let root = post_at(&pool, "root", 0, Visibility::Public).await;
    reply_at(&pool, &root, "reply", 1).await;

    sqlx::query("DELETE FROM posts WHERE id = ?1")
        .bind(root.id)
        .execute(&pool)
        .await
        .unwrap();

    let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM posts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(remaining, 0);
}

#[sqlx::test]
async fn insert_stores_the_mood_and_leaves_it_null_when_none_was_picked(pool: SqlitePool) {
    let picked = posts::insert(
        &pool,
        "shipped it",
        None,
        Visibility::Public,
        Some(Mood::Excited),
        T0,
    )
    .await
    .expect("insert");

    assert_eq!(picked.mood, Some(Mood::Excited));

    // Read back rather than trusting the returned struct: the column is what the
    // familiar reads, and these two disagreeing is exactly the bug this catches.
    let stored = posts::by_public_id(&pool, &picked.public_id)
        .await
        .unwrap()
        .expect("stored");
    assert_eq!(stored.mood, Some(Mood::Excited));

    let unpicked = post_at(&pool, "no mood on this one", 1, Visibility::Public).await;
    assert_eq!(unpicked.mood, None, "NULL is 'did not say', not 'neutral'");
}

#[sqlx::test]
async fn update_can_leave_set_and_clear_the_mood(pool: SqlitePool) {
    let post = posts::insert(
        &pool,
        "a first draft",
        None,
        Visibility::Public,
        Some(Mood::Tired),
        T0,
    )
    .await
    .expect("insert");

    // `None` — not mentioned, so untouched. Editing the body must not silently
    // wipe a mood the composer did not send.
    let untouched = posts::update(&pool, &post.public_id, Some("edited body"), None, None, T0 + 1)
        .await
        .expect("update")
        .expect("found");
    assert_eq!(untouched.post.mood, Some(Mood::Tired));
    assert_eq!(untouched.body, "edited body");

    // `Some(Some(_))` — set.
    let set = posts::update(
        &pool,
        &post.public_id,
        None,
        None,
        Some(Some(Mood::Chaos)),
        T0 + 2,
    )
    .await
    .expect("update")
    .expect("found");
    assert_eq!(set.post.mood, Some(Mood::Chaos));

    // `Some(None)` — cleared back to "did not say", which is the case a single
    // layer of Option could not express.
    let cleared = posts::update(&pool, &post.public_id, None, None, Some(None), T0 + 3)
        .await
        .expect("update")
        .expect("found");
    assert_eq!(cleared.post.mood, None);
}

#[sqlx::test]
async fn changing_only_the_mood_does_not_mark_a_post_edited(pool: SqlitePool) {
    // Mood is not part of what was published — nothing on the public site shows
    // it — so correcting one months later must not stamp the post as edited.
    let post = post_at(&pool, "a published post", 0, Visibility::Public).await;
    assert_eq!(post.edited_at, None);

    let moody = posts::update(
        &pool,
        &post.public_id,
        None,
        None,
        Some(Some(Mood::Melancholy)),
        T0 + 60_000,
    )
    .await
    .expect("update")
    .expect("found");

    assert_eq!(moody.post.mood, Some(Mood::Melancholy));
    assert_eq!(moody.post.edited_at, None, "the text is unchanged");

    // The body genuinely changing still does mark it.
    let rewritten = posts::update(
        &pool,
        &post.public_id,
        Some("different words"),
        None,
        None,
        T0 + 120_000,
    )
    .await
    .expect("update")
    .expect("found");
    assert_eq!(rewritten.post.edited_at, Some(T0 + 120_000));
}

#[sqlx::test]
async fn the_database_refuses_a_mood_that_is_not_one_of_the_seven(pool: SqlitePool) {
    // The CHECK constraint in 0003 is the backstop under the Rust enum. Without
    // it, a hand-written UPDATE in a SQLite shell could store a value that every
    // read then fails to decode.
    let post = post_at(&pool, "a post", 0, Visibility::Public).await;

    let refused = sqlx::query("UPDATE posts SET mood = 'smug' WHERE id = ?1")
        .bind(post.id)
        .execute(&pool)
        .await;

    assert!(refused.is_err(), "the CHECK constraint should have rejected it");

    // …and NULL is explicitly allowed, which the constraint has to spell out.
    sqlx::query("UPDATE posts SET mood = NULL WHERE id = ?1")
        .bind(post.id)
        .execute(&pool)
        .await
        .expect("NULL is a legal mood");
}
