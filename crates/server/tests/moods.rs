//! `GET /api/moods` — the shape the timeline is drawn from.
//!
//! The one thing worth guarding hardest is the distinction between a picked
//! `neutral` and no pick at all. It is the reason `posts.mood` is nullable, the
//! reason `PATCH` takes a doubly-optional mood, and it is exactly the kind of
//! difference that survives in the database and gets flattened on the way out.

mod common;

use axum::http::StatusCode;
use common::{app, json_request, login, send};
use serde_json::Value;
use sqlx::SqlitePool;

async fn post_with(app: &axum::Router, cookie: &str, body: &str, mood: &str, visibility: &str) {
    let json = format!(
        r#"{{"body":{},"visibility":"{visibility}","mood":{mood}}}"#,
        common::json_str(body),
    );
    let reply = send(app, json_request("POST", "/api/posts", &json, Some(cookie))).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
}

/// The count for one mood in one month's entry.
fn mood_count(month: &Value, name: &str) -> i64 {
    month["moods"]
        .as_array()
        .expect("moods")
        .iter()
        .find(|entry| entry["mood"] == name)
        .unwrap_or_else(|| panic!("{name} is missing from {month}"))["posts"]
        .as_i64()
        .expect("a count")
}

#[sqlx::test]
async fn saying_neutral_is_not_the_same_as_saying_nothing(pool: SqlitePool) {
    let app = app(pool);
    let cookie = login(&app).await;

    post_with(&app, &cookie, "nothing to report", r#""neutral""#, "public").await;
    post_with(&app, &cookie, "did not pick one", "null", "public").await;
    post_with(&app, &cookie, "also did not pick one", "null", "public").await;

    let reply = send(&app, common::get("/api/moods", Some(&cookie))).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);

    let months = reply.json()["months"].as_array().expect("months").clone();
    assert_eq!(months.len(), 1, "all three landed in one month: {}", reply.body);

    let month = &months[0];
    assert_eq!(month["total"], 3);
    assert_eq!(mood_count(month, "neutral"), 1, "an explicit neutral is a mood");
    assert_eq!(month["unsaid"], 2, "no pick is not a mood");
}

#[sqlx::test]
async fn every_mood_is_present_in_every_month_even_at_zero(pool: SqlitePool) {
    // The client indexes straight into this array to colour a stacked bar. A
    // mood that vanished in a quiet month would shift every colour after it.
    let app = app(pool);
    let cookie = login(&app).await;

    post_with(&app, &cookie, "one post", r#""tired""#, "public").await;

    let reply = send(&app, common::get("/api/moods", Some(&cookie))).await;
    let month = reply.json()["months"][0].clone();

    let names: Vec<String> = month["moods"]
        .as_array()
        .expect("moods")
        .iter()
        .map(|entry| entry["mood"].as_str().expect("a name").to_owned())
        .collect();

    // In `Mood::ALL` order, which is also the composer's picker order.
    assert_eq!(
        names,
        [
            "content",
            "contemplative",
            "tired",
            "excited",
            "melancholy",
            "chaos",
            "neutral"
        ],
    );
    assert_eq!(mood_count(&month, "tired"), 1);
    assert_eq!(mood_count(&month, "chaos"), 0);
}

#[sqlx::test]
async fn drafts_count_and_deletions_do_not(pool: SqlitePool) {
    let app = app(pool);
    let cookie = login(&app).await;

    // A draft was still written in a mood, and this page answers a question the
    // author is asking about their own writing rather than about the archive.
    post_with(&app, &cookie, "half a thought", r#""contemplative""#, "draft").await;
    post_with(&app, &cookie, "not listed", r#""excited""#, "unlisted").await;

    let doomed = common::create_post(&app, &cookie, "a mistake", "public").await;
    send(
        &app,
        common::empty_request("DELETE", &format!("/api/posts/{doomed}"), Some(&cookie)),
    )
    .await;

    let reply = send(&app, common::get("/api/moods", Some(&cookie))).await;
    let month = reply.json()["months"][0].clone();

    assert_eq!(month["total"], 2, "the deleted post should be gone: {}", reply.body);
    assert_eq!(mood_count(&month, "contemplative"), 1);
    assert_eq!(mood_count(&month, "excited"), 1);
}

#[sqlx::test]
async fn months_come_back_newest_first_and_labelled(pool: SqlitePool) {
    // Posts arrive with `now` as their timestamp, so a second month has to be
    // made directly rather than through the API.
    let app = app(pool.clone());
    let cookie = login(&app).await;
    post_with(&app, &cookie, "recent", r#""content""#, "public").await;

    // 2025-03-14T00:00:00Z and 2025-04-01T00:00:00Z.
    youwin_server::db::posts::insert(
        &pool,
        "older",
        None,
        youwin_server::db::posts::Visibility::Public,
        Some(youwin_server::mood::Mood::Melancholy),
        1_741_910_400_000,
    )
    .await
    .expect("insert");

    let reply = send(&app, common::get("/api/moods", Some(&cookie))).await;
    let months = reply.json()["months"].as_array().expect("months").clone();

    assert_eq!(months.len(), 2, "{}", reply.body);
    assert_eq!(months[1]["month"], "2025-03");
    assert_eq!(months[1]["label"], "March 2025");
    assert_eq!(mood_count(&months[1], "melancholy"), 1);

    // Newest first, like every other list in this API.
    assert!(
        months[0]["month"].as_str().unwrap() > months[1]["month"].as_str().unwrap(),
        "{}",
        reply.body,
    );
}

#[sqlx::test]
async fn an_empty_archive_returns_an_empty_list(pool: SqlitePool) {
    let app = app(pool);
    let cookie = login(&app).await;

    let reply = send(&app, common::get("/api/moods", Some(&cookie))).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(reply.json()["months"].as_array().expect("months").is_empty());
}

#[sqlx::test]
async fn moods_are_behind_the_session_guard(pool: SqlitePool) {
    // Mood never renders on youwin.dev. A route that leaked it without a session
    // would be that rule undone, on the one host where nothing is public.
    let reply = send(&app(pool), common::get("/api/moods", None)).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
}
