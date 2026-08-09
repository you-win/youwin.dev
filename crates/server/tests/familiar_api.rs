//! The familiar on the authoring host: the resting pet, and the pet as a draft
//! would leave it.
//!
//! The state machine itself is covered by unit tests and `tests/familiar.rs`.
//! What is worth testing here is the wiring, and specifically the two ways it
//! could go quietly wrong: a hypothetical leaking into the stored snapshot, and
//! a real post failing to.

mod common;

use axum::{Router, http::StatusCode};
use common::{create_post, get, json_request, json_str, login, send};
use sqlx::SqlitePool;

/// `POST /api/familiar/draft` with a body and nothing else picked.
fn draft_request(body: &str, cookie: &str) -> axum::http::Request<axum::body::Body> {
    json_request(
        "POST",
        "/api/familiar/draft",
        &format!(r#"{{"body":{}}}"#, json_str(body)),
        Some(cookie),
    )
}

async fn familiar(app: &Router, cookie: &str) -> serde_json::Value {
    let reply = send(app, get("/api/familiar", Some(cookie))).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    reply.json()
}

#[sqlx::test]
async fn both_familiar_routes_require_a_session(pool: SqlitePool) {
    let app = common::app(pool);

    for request in [
        get("/api/familiar", None),
        json_request("POST", "/api/familiar/draft", r#"{"body":"hi"}"#, None),
    ] {
        let uri = request.uri().to_string();
        let reply = send(&app, request).await;
        assert_eq!(
            reply.status,
            StatusCode::UNAUTHORIZED,
            "{uri} is reachable without a session",
        );
    }
}

#[sqlx::test]
async fn the_resting_pet_arrives_drawn_and_described(pool: SqlitePool) {
    let app = common::app(pool);
    let cookie = login(&app).await;

    let pet = familiar(&app, &cookie).await;

    assert_eq!(pet["posts"], 0);
    assert_eq!(pet["stage"], "egg");

    // The picture, as lines rather than one blob — the client centres them, and
    // padding by character count would be wrong for glyphs whose width it
    // cannot know.
    let lines = pet["lines"].as_array().expect("lines");
    assert!(!lines.is_empty(), "{pet}");
    assert!(lines.iter().all(|line| line.is_string()), "{pet}");

    let description = pet["description"].as_str().expect("description");
    assert!(description.starts_with("The familiar"), "{description}");
    assert!(!description.contains('('), "no glyphs in the label: {description}");

    // An egg has somewhere to grow, and says how far along it is.
    assert_eq!(pet["growth"]["toward"], "hatchling");
    assert_eq!(pet["growth"]["percent"], 0);
}

#[sqlx::test]
async fn a_draft_previews_the_pet_it_would_produce(pool: SqlitePool) {
    let app = common::app(pool);
    let cookie = login(&app).await;

    create_post(&app, &cookie, "the first note, about rust", "public").await;

    let before = familiar(&app, &cookie).await;
    assert_eq!(before["posts"], 1);

    let reply = send(
        &app,
        draft_request("another deploy, another rust refactor", &cookie),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);

    let previewed = reply.json();
    assert_eq!(previewed["posts"], 2, "the draft counts as a post");
    assert_eq!(previewed["form"], "hexapod", "tech posts make a hexapod");

    // The one that matters. A hypothetical is not a fact about the archive, and
    // the composer asks this on every pause in typing — if it wrote to the
    // snapshot the pet would go on reflecting a post that was never made.
    let after = familiar(&app, &cookie).await;
    assert_eq!(after["posts"], 1, "the preview leaked into the snapshot");
}

#[sqlx::test]
async fn a_picked_mood_reaches_the_previewed_face(pool: SqlitePool) {
    let app = common::app(pool);
    let cookie = login(&app).await;

    // Text that reads unmistakably as one thing, filed under another. The picker
    // gets the last word here exactly as it does on a stored post.
    let reply = send(
        &app,
        json_request(
            "POST",
            "/api/familiar/draft",
            &format!(
                r#"{{"body":{},"mood":"melancholy"}}"#,
                json_str("an amazing incredible breakthrough, shipped at last"),
            ),
            Some(&cookie),
        ),
    )
    .await;

    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["mood"], "melancholy");

    // And with nothing picked, the same words read as what they say.
    let inferred = send(
        &app,
        draft_request("an amazing incredible breakthrough, shipped at last", &cookie),
    )
    .await;
    assert_eq!(inferred.json()["mood"], "excited");
}

#[sqlx::test]
async fn nothing_unpublished_previews_as_a_change(pool: SqlitePool) {
    let app = common::app(pool);
    let cookie = login(&app).await;

    create_post(&app, &cookie, "the first note, about rust", "public").await;
    let resting = familiar(&app, &cookie).await;

    // Only public posts feed the pet. A draft or an unlisted note previewing as
    // growth would be the composer promising something that will not happen.
    for visibility in ["draft", "unlisted"] {
        let reply = send(
            &app,
            json_request(
                "POST",
                "/api/familiar/draft",
                &format!(
                    r#"{{"body":{},"visibility":"{visibility}"}}"#,
                    json_str("a long note about hiking through wet forest"),
                ),
                Some(&cookie),
            ),
        )
        .await;

        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        let previewed = reply.json();
        assert_eq!(previewed["posts"], resting["posts"], "{visibility} moved the pet");
        assert_eq!(previewed["form"], resting["form"], "{visibility} moved the pet");
    }

    // Neither does an empty box, which is not an error — asking what a
    // half-written note would do is not asking to publish it.
    let blank = send(&app, draft_request("   \n  ", &cookie)).await;
    assert_eq!(blank.status, StatusCode::OK, "{}", blank.body);
    assert_eq!(blank.json()["posts"], resting["posts"]);
}

#[sqlx::test]
async fn what_the_preview_promises_is_what_posting_delivers(pool: SqlitePool) {
    let app = common::app(pool);
    let cookie = login(&app).await;

    create_post(&app, &cookie, "a first note about rust", "public").await;
    familiar(&app, &cookie).await;

    // The composer is a promise about what this post will do, so the promise has
    // to survive the post being made. It did not: invalidating after a write by
    // dropping the snapshot handed the recompute a `previous` of None, which is
    // the cold-start path — and cold start applies no burst, so a draft
    // previewed as `hyper` landed as a flat `active` every single time.
    let body = "an amazing incredible breakthrough, shipped at last";
    let promised = send(&app, draft_request(body, &cookie)).await.json();

    create_post(&app, &cookie, body, "public").await;
    let delivered = familiar(&app, &cookie).await;

    assert_eq!(delivered["posts"], promised["posts"]);
    assert_eq!(delivered["mood"], promised["mood"]);
    assert_eq!(
        delivered["level"], promised["level"],
        "previewed as {} and landed as {}",
        promised["level"], delivered["level"],
    );
}

#[sqlx::test]
async fn a_real_post_moves_the_pet_without_waiting_out_the_ttl(pool: SqlitePool) {
    let app = common::app(pool);
    let cookie = login(&app).await;

    // Warm the snapshot first, so there is something stale to invalidate. This
    // is the whole reason the authoring host holds its own: the public site's
    // five-minute TTL is matched to the edge cache in front of it, and the same
    // wait next to the composer would mean posting appeared to do nothing.
    assert_eq!(familiar(&app, &cookie).await["posts"], 0);

    create_post(&app, &cookie, "the first note, about rust", "public").await;
    assert_eq!(familiar(&app, &cookie).await["posts"], 1, "still stale");

    let second = create_post(&app, &cookie, "a second note, about rust", "public").await;
    assert_eq!(familiar(&app, &cookie).await["posts"], 2);

    // Deleting is a change too, and forgetting has to cover it.
    let deleted = send(
        &app,
        common::empty_request("DELETE", &format!("/api/posts/{second}"), Some(&cookie)),
    )
    .await;
    assert_eq!(deleted.status, StatusCode::OK, "{}", deleted.body);
    assert_eq!(familiar(&app, &cookie).await["posts"], 1);
}
