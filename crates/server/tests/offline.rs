//! The idempotency key that makes a queued post safe to flush twice.
//!
//! The composer queues posts written offline and retries them when the
//! connection returns. A retry cannot tell "the request never arrived" from
//! "the reply never came back", so without a key the honest choices are posting
//! twice or losing the post. These tests pin the third option.

mod common;

use axum::http::StatusCode;
use common::{app, json_request, login, send};
use sqlx::SqlitePool;

/// A create request carrying a key, spelled out rather than built by a helper so
/// what is on the wire is visible in the test.
fn queued(body: &str, key: &str) -> String {
    format!(
        r#"{{"body":{},"visibility":"public","idempotency_key":"{key}"}}"#,
        common::json_str(body),
    )
}

#[sqlx::test]
async fn the_same_key_twice_writes_one_post(pool: SqlitePool) {
    let app = app(pool);
    let cookie = login(&app).await;

    let first = send(
        &app,
        json_request("POST", "/api/posts", &queued("written on a train", "k-1"), Some(&cookie)),
    )
    .await;
    assert_eq!(first.status, StatusCode::CREATED, "{}", first.body);

    // The flush that could not tell whether the first one landed.
    let second = send(
        &app,
        json_request("POST", "/api/posts", &queued("written on a train", "k-1"), Some(&cookie)),
    )
    .await;

    // 200, not 201: nothing was created, and the client is entitled to know its
    // earlier attempt had already arrived.
    assert_eq!(second.status, StatusCode::OK, "{}", second.body);
    assert_eq!(
        second.json()["id"], first.json()["id"],
        "a replay must return the post the first attempt wrote",
    );

    let feed = send(&app, common::get("/api/posts", Some(&cookie))).await;
    assert_eq!(
        feed.json()["posts"].as_array().unwrap().len(),
        1,
        "one post, not two: {}",
        feed.body,
    );
}

#[sqlx::test]
async fn a_replay_returns_the_original_even_when_the_body_differs(pool: SqlitePool) {
    // Not a case the client produces, but the key is what identifies the
    // request — if the body were consulted, an edit made between two flush
    // attempts would post twice.
    let app = app(pool);
    let cookie = login(&app).await;

    let first = send(
        &app,
        json_request("POST", "/api/posts", &queued("the original", "k-2"), Some(&cookie)),
    )
    .await;
    let second = send(
        &app,
        json_request("POST", "/api/posts", &queued("something else", "k-2"), Some(&cookie)),
    )
    .await;

    assert_eq!(second.status, StatusCode::OK);
    assert_eq!(second.json()["id"], first.json()["id"]);
    assert_eq!(second.json()["body"], "the original");
}

#[sqlx::test]
async fn different_keys_are_different_posts(pool: SqlitePool) {
    let app = app(pool);
    let cookie = login(&app).await;

    for key in ["a", "b", "c"] {
        let reply = send(
            &app,
            json_request("POST", "/api/posts", &queued("same words", key), Some(&cookie)),
        )
        .await;
        assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    }

    // Identical bodies, so nothing but the key distinguishes them — which is the
    // point. Writing the same sentence three times is allowed.
    let feed = send(&app, common::get("/api/posts", Some(&cookie))).await;
    assert_eq!(feed.json()["posts"].as_array().unwrap().len(), 3);
}

#[sqlx::test]
async fn posts_without_a_key_are_never_deduplicated(pool: SqlitePool) {
    // Every post written before this feature has a NULL key, and so does every
    // post made online. SQLite allows any number of NULLs in a unique index —
    // this asserts that the partial index in 0004 did not change that.
    let app = app(pool);
    let cookie = login(&app).await;

    common::create_post(&app, &cookie, "identical", "public").await;
    common::create_post(&app, &cookie, "identical", "public").await;

    let feed = send(&app, common::get("/api/posts", Some(&cookie))).await;
    assert_eq!(feed.json()["posts"].as_array().unwrap().len(), 2);
}

#[sqlx::test]
async fn an_empty_key_is_treated_as_no_key(pool: SqlitePool) {
    // `""` is what a client bug produces, and it must not become a key that
    // every subsequent post collides with — which would silently stop posting
    // working altogether.
    let app = app(pool);
    let cookie = login(&app).await;

    for _ in 0..2 {
        let reply = send(
            &app,
            json_request(
                "POST",
                "/api/posts",
                r#"{"body":"no key at all","visibility":"public","idempotency_key":"  "}"#,
                Some(&cookie),
            ),
        )
        .await;
        assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    }

    let feed = send(&app, common::get("/api/posts", Some(&cookie))).await;
    assert_eq!(feed.json()["posts"].as_array().unwrap().len(), 2);
}

#[sqlx::test]
async fn an_absurd_key_is_refused_rather_than_indexed(pool: SqlitePool) {
    let app = app(pool);
    let cookie = login(&app).await;

    let key = "k".repeat(65);
    let reply = send(
        &app,
        json_request("POST", "/api/posts", &queued("too long a key", &key), Some(&cookie)),
    )
    .await;

    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", reply.body);
    assert_eq!(reply.json()["error"]["code"], "invalid");
}

#[sqlx::test]
async fn a_key_stays_spent_after_the_post_is_deleted(pool: SqlitePool) {
    // Delete on one device, lose signal on another, and the queued copy flushes
    // again. It must not come back from the dead. The key is spent the moment it
    // writes a row, and soft deletion keeps that row.
    let app = app(pool);
    let cookie = login(&app).await;

    let first = send(
        &app,
        json_request("POST", "/api/posts", &queued("second thoughts", "k-3"), Some(&cookie)),
    )
    .await;
    let id = first.json()["id"].as_str().expect("id").to_owned();

    let removed = send(
        &app,
        common::empty_request("DELETE", &format!("/api/posts/{id}"), Some(&cookie)),
    )
    .await;
    assert_eq!(removed.status, StatusCode::OK, "{}", removed.body);

    let replay = send(
        &app,
        json_request("POST", "/api/posts", &queued("second thoughts", "k-3"), Some(&cookie)),
    )
    .await;

    // A definite answer, and specifically not a 201. The outbox drops the item
    // on any response at all — only a network failure is worth retrying — so
    // what matters here is that no second post exists.
    assert_ne!(replay.status, StatusCode::CREATED, "{}", replay.body);

    let feed = send(&app, common::get("/api/posts", Some(&cookie))).await;
    assert!(
        feed.json()["posts"].as_array().unwrap().is_empty(),
        "the deleted post must not have been rewritten: {}",
        feed.body,
    );
}

#[sqlx::test]
async fn a_queued_reply_keys_the_same_way(pool: SqlitePool) {
    let app = app(pool);
    let cookie = login(&app).await;
    let parent = common::create_post(&app, &cookie, "the post being answered", "public").await;

    let body = format!(
        r#"{{"body":"answered later","parent_id":"{parent}","visibility":"public","idempotency_key":"k-4"}}"#
    );

    let first = send(&app, json_request("POST", "/api/posts", &body, Some(&cookie))).await;
    assert_eq!(first.status, StatusCode::CREATED, "{}", first.body);

    let second = send(&app, json_request("POST", "/api/posts", &body, Some(&cookie))).await;
    assert_eq!(second.status, StatusCode::OK, "{}", second.body);
    assert_eq!(second.json()["id"], first.json()["id"]);

    let thread = send(&app, common::get(&format!("/api/posts/{parent}"), Some(&cookie))).await;
    assert_eq!(
        thread.json()["post"]["reply_count"], 1,
        "one reply, not two: {}",
        thread.body,
    );
}
