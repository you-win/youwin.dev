//! One test per statement in `db::search` and `db::tags`, plus the FTS triggers.
//!
//! The triggers are the part with no compiler and no type behind them: if
//! `posts_fts_au` stopped firing, every existing search would keep working and
//! only edits would silently stop being findable. So each trigger is asserted
//! through the public API that fires it, not by poking the index directly.

use sqlx::SqlitePool;
use youwin_server::db::{
    posts::{self, Cursor, Post, Visibility},
    search, tags,
};

const T0: i64 = 1_786_000_000_000;
const PAGE: i64 = 20;

async fn post_at(pool: &SqlitePool, body: &str, minutes: i64, visibility: Visibility) -> Post {
    posts::insert(pool, body, None, visibility, T0 + minutes * 60_000)
        .await
        .expect("insert")
}

/// Runs a search the way a route does: sanitize, then query.
async fn find(pool: &SqlitePool, typed: &str) -> Vec<search::Hit> {
    let query = search::fts_query(typed).expect("a searchable query");
    search::public(pool, &query, Cursor::START, PAGE)
        .await
        .expect("search")
        .0
}

#[sqlx::test]
async fn fts5_is_compiled_into_the_bundled_sqlite(pool: SqlitePool) {
    // Not a property of this code but of how `libsqlite3-sys` was built, which a
    // dependency bump could change without anything else failing. Everything in
    // this file would then fail at once and this line explains why.
    let modules: Vec<String> = sqlx::query_scalar("PRAGMA module_list")
        .fetch_all(&pool)
        .await
        .expect("module_list");

    assert!(modules.iter().any(|m| m == "fts5"), "got: {modules:?}");
}

#[sqlx::test]
async fn a_new_post_is_findable_and_the_stemmer_is_on(pool: SqlitePool) {
    post_at(&pool, "reading about capybaras today", 0, Visibility::Public).await;

    let hits = find(&pool, "capybaras").await;
    assert_eq!(hits.len(), 1);

    // Porter stemming, from the tokenizer in 0002. Without it the singular
    // would miss entirely, which is the single most common failed search.
    assert_eq!(find(&pool, "capybara").await.len(), 1, "stemming is off");
    assert_eq!(find(&pool, "read").await.len(), 1, "stemming is off");

    // Both words must appear: tokens are ANDed.
    assert_eq!(find(&pool, "capybaras today").await.len(), 1);
    assert_eq!(find(&pool, "capybaras yesterday").await.len(), 0);
}

#[sqlx::test]
async fn editing_a_post_reindexes_it(pool: SqlitePool) {
    // The `posts_fts_au` trigger. Without it the old text stays findable forever
    // and the new text is never findable at all.
    let post = post_at(&pool, "about herons", 0, Visibility::Public).await;

    posts::update(&pool, &post.public_id, Some("about egrets"), None, T0 + 1)
        .await
        .expect("update");

    assert_eq!(find(&pool, "egrets").await.len(), 1, "new text is not indexed");
    assert_eq!(find(&pool, "herons").await.len(), 0, "old text is still indexed");
}

#[sqlx::test]
async fn changing_only_visibility_does_not_disturb_the_index(pool: SqlitePool) {
    // `AFTER UPDATE OF body_text` is what keeps this cheap. It is also the sort
    // of thing that regresses silently, since the wrong version still works.
    let post = post_at(&pool, "about herons", 0, Visibility::Draft).await;
    assert_eq!(find(&pool, "herons").await.len(), 0, "drafts are not public");

    posts::update(&pool, &post.public_id, None, Some(Visibility::Public), T0 + 1)
        .await
        .expect("publish");

    assert_eq!(find(&pool, "herons").await.len(), 1);
}

#[sqlx::test]
async fn search_hides_what_the_public_site_hides(pool: SqlitePool) {
    let public = post_at(&pool, "shared marmots", 0, Visibility::Public).await;
    post_at(&pool, "quiet marmots", 1, Visibility::Unlisted).await;
    post_at(&pool, "unfinished marmots", 2, Visibility::Draft).await;
    let deleted = post_at(&pool, "removed marmots", 3, Visibility::Public).await;
    posts::soft_delete(&pool, &deleted.public_id, T0 + 4)
        .await
        .expect("delete");

    let hits = find(&pool, "marmots").await;
    assert_eq!(hits.len(), 1, "got: {:?}", hits.iter().map(|h| &h.post.body_text).collect::<Vec<_>>());
    assert_eq!(hits[0].post.public_id, public.public_id);

    // Unlisted is reachable by link but must not be discoverable — which is the
    // entire difference between `unlisted` and `public`.
    let authored = search::authored(&pool, &search::fts_query("marmots").unwrap(), Cursor::START, PAGE)
        .await
        .expect("authored search");
    assert_eq!(authored.0.len(), 3, "the authoring side sees everything but deletions");
}

#[sqlx::test]
async fn hits_carry_a_snippet_with_the_matched_term_marked(pool: SqlitePool) {
    post_at(
        &pool,
        "a long enough sentence that the interesting word sits in the middle of it somewhere",
        0,
        Visibility::Public,
    )
    .await;

    let hits = find(&pool, "interesting").await;
    let runs = search::segments(&hits[0].snippet);

    let marked: Vec<&str> = runs.iter().filter(|(hit, _)| *hit).map(|(_, t)| *t).collect();
    assert_eq!(marked, vec!["interesting"], "got runs: {runs:?}");

    // Nothing HTML-shaped comes out of the database — that is why the markers
    // are control characters.
    assert!(!hits[0].snippet.contains('<'), "{:?}", hits[0].snippet);
}

#[sqlx::test]
async fn search_paginates_with_the_same_cursor_as_the_feed(pool: SqlitePool) {
    for i in 0..3 {
        post_at(&pool, &format!("otters number {i}"), i, Visibility::Public).await;
    }

    let query = search::fts_query("otters").unwrap();
    let (first, next) = search::public(&pool, &query, Cursor::START, 2).await.unwrap();
    assert_eq!(first.len(), 2);
    assert_eq!(first[0].post.body_text, "otters number 2", "newest first");

    let (second, last) = search::public(&pool, &query, next.expect("a next page"), 2)
        .await
        .unwrap();
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].post.body_text, "otters number 0");
    assert!(last.is_none(), "the last page has no cursor");
}

#[sqlx::test]
async fn reply_counts_come_back_on_hits(pool: SqlitePool) {
    let root = post_at(&pool, "starlings roost here", 0, Visibility::Public).await;
    posts::insert(&pool, "one more thing", Some(root.id), Visibility::Public, T0 + 60_000)
        .await
        .expect("reply");

    let hits = find(&pool, "starlings").await;
    assert_eq!(hits[0].reply_count, 1);
}

// ---------------------------------------------------------------------------
// Tags
// ---------------------------------------------------------------------------

#[sqlx::test]
async fn posting_records_its_hashtags(pool: SqlitePool) {
    let post = post_at(&pool, "shipping #Rust and #web-dev today", 0, Visibility::Public).await;

    // The link and the index agree — the property the single-pass extraction in
    // `render::markdown` exists to guarantee.
    assert!(post.body_html.contains(r#"href="/t/rust""#), "{}", post.body_html);

    let (rows, _) = tags::feed_page(&pool, "rust", Cursor::START, PAGE).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].post.public_id, post.public_id);

    // Case-insensitive lookup, canonicalized on the way in.
    let (upper, _) = tags::feed_page(&pool, "RUST", Cursor::START, PAGE).await.unwrap();
    assert_eq!(upper.len(), 1);

    assert_eq!(tags::display_name(&pool, "rust").await.unwrap().as_deref(), Some("Rust"));
    assert_eq!(tags::display_name(&pool, "nothing").await.unwrap(), None);
}

#[sqlx::test]
async fn editing_a_post_replaces_its_tags(pool: SqlitePool) {
    let post = post_at(&pool, "about #herons", 0, Visibility::Public).await;

    posts::update(&pool, &post.public_id, Some("about #egrets"), None, T0 + 1)
        .await
        .expect("update");

    let (dropped, _) = tags::feed_page(&pool, "herons", Cursor::START, PAGE).await.unwrap();
    assert!(dropped.is_empty(), "the removed tag still lists the post");

    let (added, _) = tags::feed_page(&pool, "egrets", Cursor::START, PAGE).await.unwrap();
    assert_eq!(added.len(), 1);
}

#[sqlx::test]
async fn tag_pages_hide_drafts_and_deletions_and_include_replies(pool: SqlitePool) {
    let root = post_at(&pool, "a thread about #bats", 0, Visibility::Public).await;
    posts::insert(
        &pool,
        "more on #bats",
        Some(root.id),
        Visibility::Public,
        T0 + 60_000,
    )
    .await
    .expect("reply");
    post_at(&pool, "unfinished #bats", 2, Visibility::Draft).await;
    let deleted = post_at(&pool, "removed #bats", 3, Visibility::Public).await;
    posts::soft_delete(&pool, &deleted.public_id, T0 + 4).await.unwrap();

    let (rows, _) = tags::feed_page(&pool, "bats", Cursor::START, PAGE).await.unwrap();

    // A hashtag on a reply is still a hashtag: two visible posts, not one.
    assert_eq!(rows.len(), 2, "got: {:?}", rows.iter().map(|r| &r.post.body_text).collect::<Vec<_>>());
    assert!(rows.iter().any(|r| r.post.parent_id.is_some()), "the reply is missing");
}

#[sqlx::test]
async fn tag_pages_paginate(pool: SqlitePool) {
    for i in 0..3 {
        post_at(&pool, &format!("#owls number {i}"), i, Visibility::Public).await;
    }

    let (first, next) = tags::feed_page(&pool, "owls", Cursor::START, 2).await.unwrap();
    assert_eq!(first.len(), 2);
    let (second, last) = tags::feed_page(&pool, "owls", next.expect("next"), 2).await.unwrap();
    assert_eq!(second.len(), 1);
    assert!(last.is_none());
}

#[sqlx::test]
async fn rerender_rebuilds_derived_columns_without_marking_anything_edited(pool: SqlitePool) {
    let post = post_at(&pool, "notes on #rust and mist", 0, Visibility::Public).await;

    // Exactly the state upgrading to 0002 leaves behind: `body` is right, the
    // rendered HTML predates hashtag linking, and no tag rows exist at all.
    sqlx::query("UPDATE posts SET body_html = '<p>stale</p>' WHERE id = ?1")
        .bind(post.id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM post_tags").execute(&pool).await.unwrap();

    let before: (i64, Option<i64>) =
        sqlx::query_as("SELECT updated_at, edited_at FROM posts WHERE id = ?1")
            .bind(post.id)
            .fetch_one(&pool)
            .await
            .unwrap();

    let result = posts::rerender_all(&pool).await.expect("rerender");
    assert_eq!(result.scanned, 1);
    assert_eq!(result.rewritten, 1);

    let stored = posts::by_public_id(&pool, &post.public_id).await.unwrap().unwrap();
    assert!(stored.body_html.contains(r#"href="/t/rust""#), "{}", stored.body_html);

    let (tagged, _) = tags::feed_page(&pool, "rust", Cursor::START, PAGE).await.unwrap();
    assert_eq!(tagged.len(), 1, "tags were not rebuilt");

    // A re-render is not an edit. Marking an archive as edited because the
    // renderer changed would be a false claim about the post's history.
    let after: (i64, Option<i64>) =
        sqlx::query_as("SELECT updated_at, edited_at FROM posts WHERE id = ?1")
            .bind(post.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(after, before);

    // Idempotent: a second pass finds nothing to do.
    let again = posts::rerender_all(&pool).await.unwrap();
    assert_eq!((again.scanned, again.rewritten), (1, 0));
}

#[sqlx::test]
async fn export_carries_every_post_with_its_tags_and_thread_links(pool: SqlitePool) {
    let root = post_at(&pool, "a thread about #bats", 0, Visibility::Public).await;
    posts::insert(&pool, "more", Some(root.id), Visibility::Public, T0 + 60_000)
        .await
        .expect("reply");
    let gone = post_at(&pool, "removed", 2, Visibility::Public).await;
    posts::soft_delete(&pool, &gone.public_id, T0 + 3).await.unwrap();

    let rows = posts::export_all(&pool).await.expect("export");
    assert_eq!(rows.len(), 3, "a backup that drops deletions is not a backup");

    let reply = rows.iter().find(|r| r.body == "more").expect("the reply");
    // Rowids are translated out, so the archive stands on its own.
    assert_eq!(reply.parent_public_id.as_deref(), Some(root.public_id.as_str()));
    assert_eq!(reply.root_public_id, root.public_id);

    let deleted = rows.iter().find(|r| r.body == "removed").expect("the deletion");
    assert!(deleted.deleted_at.is_some());

    let tagged = tags::by_post(&pool).await.expect("tags");
    assert_eq!(tagged.len(), 1);
    assert_eq!(tagged[0].1, "bats");
}

#[sqlx::test]
async fn the_tag_list_counts_only_visible_posts(pool: SqlitePool) {
    post_at(&pool, "#rust one", 0, Visibility::Public).await;
    post_at(&pool, "#rust two", 1, Visibility::Public).await;
    post_at(&pool, "#sqlite one", 2, Visibility::Public).await;
    post_at(&pool, "#secret one", 3, Visibility::Draft).await;

    let all = tags::all(&pool).await.unwrap();
    let names: Vec<(&str, i64)> = all.iter().map(|t| (t.tag.as_str(), t.posts)).collect();

    // Most-used first, and a tag with nothing public to show does not appear at
    // all — which is also what stops a draft leaking a tag name.
    assert_eq!(names, vec![("rust", 2), ("sqlite", 1)]);
}
