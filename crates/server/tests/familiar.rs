//! The familiar end to end: its one statement, its snapshot, and the two pages
//! it appears on.
//!
//! The statement in `db::familiar` is runtime-checked like every other, so these
//! tests are the only thing between a renamed column and an empty pet on the
//! front page — and an empty pet renders perfectly happily as an egg, which is
//! exactly the kind of failure nobody notices. Row *shape* is asserted, not just
//! that the query returned.

use std::path::Path;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use sqlx::SqlitePool;
use tower::ServiceExt as _;
use youwin_server::{
    db::{
        self,
        posts::{Cursor, Post, Visibility},
    },
    familiar::{Familiar, Stage, cache::TTL_MILLIS},
    mood::Mood,
    public::{self, assets::Assets},
};

/// 2026-08-01T00:00:00Z.
const T0: i64 = 1_785_888_000_000;
const HOUR: i64 = 3_600_000;

async fn post_at(pool: &SqlitePool, body: &str, hours: i64, visibility: Visibility) -> Post {
    db::posts::insert(pool, body, None, visibility, None, T0 + hours * HOUR)
        .await
        .expect("insert")
}

fn app(pool: SqlitePool) -> Router {
    public::router(
        pool,
        // Stands in for the Vite manifest lookup, which needs a real frontend
        // build on disk and has nothing to do with these routes.
        Assets {
            css: "/assets/test.css".to_owned(),
        },
        "https://youwin.dev".to_owned(),
        // Never read: no test here requests /assets.
        Path::new("web/dist/public"),
    )
}

async fn get(app: &Router, uri: &str) -> (StatusCode, String) {
    let response = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .expect("router");

    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");

    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[sqlx::test]
async fn all_returns_both_columns_oldest_first(pool: SqlitePool) {
    post_at(&pool, "second, about a *hike*", 1, Visibility::Public).await;
    post_at(&pool, "first, about `rust`", 0, Visibility::Public).await;

    let fed = db::familiar::all(&pool).await.expect("query");

    assert_eq!(fed.len(), 2);
    assert_eq!(fed[0].created_at, T0, "oldest first — compute binary-searches this");
    assert_eq!(fed[1].created_at, T0 + HOUR);

    // The plaintext projection, not the markdown source and not the HTML: the
    // keyword tables match against prose, and `*hike*` would not be a hike.
    assert_eq!(fed[0].body_text, "first, about rust");
    assert_eq!(fed[1].body_text, "second, about a hike");
}

#[sqlx::test]
async fn replies_feed_the_familiar_but_nothing_unpublished_does(pool: SqlitePool) {
    let root = post_at(&pool, "a public root", 0, Visibility::Public).await;

    // A reply is something that was sat down and written, so it counts.
    db::posts::insert(&pool, "a public reply", Some(root.id), Visibility::Public, None, T0 + HOUR)
        .await
        .expect("reply");

    // These must not. Counting an unlisted post would let a visitor infer from
    // the number on the page that one exists, which is the only thing `unlisted`
    // protects.
    post_at(&pool, "unlisted", 2, Visibility::Unlisted).await;
    post_at(&pool, "draft", 3, Visibility::Draft).await;

    let deleted = post_at(&pool, "deleted", 4, Visibility::Public).await;
    db::posts::soft_delete(&pool, &deleted.public_id, T0 + 5 * HOUR)
        .await
        .expect("delete");

    let fed = db::familiar::all(&pool).await.expect("query");
    let bodies: Vec<_> = fed.iter().map(|post| post.body_text.as_str()).collect();
    assert_eq!(bodies, ["a public root", "a public reply"]);
}

#[sqlx::test]
async fn a_picked_mood_reaches_the_pets_face_and_overrides_the_text(pool: SqlitePool) {
    // Text that reads unmistakably as one thing, filed under another. The whole
    // point of the picker is that the writer gets the last word.
    db::posts::insert(
        &pool,
        "an amazing incredible breakthrough, shipped at last",
        None,
        Visibility::Public,
        Some(Mood::Melancholy),
        T0,
    )
    .await
    .expect("insert");

    let reading = Familiar::new().read(&pool, T0).await.expect("read");
    assert_eq!(reading.state.mood, Mood::Melancholy);

    // The same post with nothing picked reads as what it says.
    let inferred = db::familiar::all(&pool).await.expect("query");
    assert_eq!(inferred[0].mood, Some(Mood::Melancholy), "the column round-trips");
}

#[sqlx::test]
async fn an_empty_archive_is_an_egg_rather_than_an_error(pool: SqlitePool) {
    let reading = Familiar::new().read(&pool, T0).await.expect("read");

    assert_eq!(reading.state.stage, Stage::Egg);
    assert_eq!(reading.vitals.posts, 0);
    assert_eq!(reading.moods, vec![]);
}

#[sqlx::test]
async fn the_snapshot_is_held_for_its_ttl_and_then_catches_up(pool: SqlitePool) {
    post_at(&pool, "the first note, about rust", 0, Visibility::Public).await;

    let familiar = Familiar::new();
    let first = familiar.read(&pool, T0).await.expect("read");
    assert_eq!(first.vitals.posts, 1);

    post_at(&pool, "a second note, about a hike", 0, Visibility::Public).await;

    // Inside the window the snapshot stands, so the new post is not visible yet
    // — which is the same five minutes the page itself is cached for.
    let held = familiar.read(&pool, T0 + TTL_MILLIS - 1).await.expect("read");
    assert_eq!(held.vitals.posts, 1, "the snapshot should still be the first one");

    let caught_up = familiar.read(&pool, T0 + TTL_MILLIS).await.expect("read");
    assert_eq!(caught_up.vitals.posts, 2);
}

#[sqlx::test]
async fn the_feed_carries_the_familiar_on_the_first_page_only(pool: SqlitePool) {
    for hour in 0..25 {
        post_at(&pool, "another note about rust and deploys", hour, Visibility::Public).await;
    }

    let app = app(pool);

    let (status, first_page) = get(&app, "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(first_page.contains(r#"href="/familiar""#), "no familiar on the feed");

    // Page two. The pet reads the whole archive and says nothing about the
    // twenty posts under it, so repeating it down the pagination would be copies
    // of one fact.
    let cursor = Cursor {
        created_at: T0 + 5 * HOUR,
        id: 5,
    };
    let (status, second_page) = get(&app, &format!("/?before={}", cursor.encode())).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!second_page.contains(r#"href="/familiar""#), "{second_page}");
}

#[sqlx::test]
async fn the_familiar_page_renders_the_pet_and_its_sheet(pool: SqlitePool) {
    for hour in 0..12 {
        post_at(&pool, "shipped another rust deploy, no bugs", hour, Visibility::Public).await;
    }

    let (status, body) = get(&app(pool), "/familiar").await;
    assert_eq!(status, StatusCode::OK);

    // The pet itself, in a <pre> with a description rather than as bare glyphs a
    // screen reader would spell out.
    assert!(body.contains("<pre role=\"img\" aria-label=\"The familiar:"), "{body}");
    assert!(body.contains("hexapod"), "tech posts make a hexapod: {body}");

    // The sheet under it.
    for expected in ["vitals", "diet", "character sheet", "VIT", "MAG", "█"] {
        assert!(body.contains(expected), "missing {expected:?} in {body}");
    }

    assert!(body.contains("<link rel=\"canonical\" href=\"https://youwin.dev/familiar\">"), "{body}");

    // Twelve posts is under the sample a trait is read from, so this archive has
    // no character yet and the line is absent rather than empty.
    assert!(!body.contains("traits:"), "{body}");
}

#[sqlx::test]
async fn a_character_reaches_the_page_once_there_is_one_to_read(pool: SqlitePool) {
    // Dated backwards from the fixture epoch for the same reason the speech test
    // is: these routes read the real clock, and posts in the future are invisible
    // to `compute`.
    //
    // Twenty two-word notes, one a day, always at the same hour. Terse by a wide
    // margin, and with hours sharp enough that nothing is said about them.
    for day in 0..20 {
        post_at(&pool, "a note", -(30 - day) * 24, Visibility::Public).await;
    }

    let (status, body) = get(&app(pool), "/familiar").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("traits: terse"), "{body}");
    assert!(!body.contains("scattered"), "this archive keeps its hours: {body}");
}

#[sqlx::test]
async fn the_pet_speaks_on_both_public_surfaces_when_it_has_something_to_say(pool: SqlitePool) {
    // Twenty days of a daily note, all of it comfortably behind us, and then
    // nothing. These routes read the real clock, so the archive has to be dated
    // *backwards* from the fixture epoch — posts in the future are invisible to
    // `compute`, which would leave a very different archive than the one this
    // test means to describe.
    //
    // A silence of a fortnight-plus against a daily rhythm is unlike anything in
    // this writer's history, which is the whole condition for saying anything.
    // It only gets more true as real time moves on.
    for day in 0..20 {
        let days_before_epoch = 30 - day;
        post_at(&pool, "another note about rust", -days_before_epoch * 24, Visibility::Public).await;
    }

    let app = app(pool);
    let (_, feed) = get(&app, "/").await;
    let (_, sheet) = get(&app, "/familiar").await;

    // Both surfaces draw from one snapshot, so either both carry the line or
    // neither does — a widget that disagrees with the page it links to is the
    // failure this shares a `Reading` to prevent.
    let on_feed = feed.contains("it has not been fed");
    let on_sheet = sheet.contains("it has not been fed");
    assert_eq!(on_feed, on_sheet, "the widget and the sheet disagree");
    assert!(on_feed, "twenty days of daily notes and then nothing: {feed}");

    // And it is the pet talking, never the page addressing whoever is reading.
    for page in [&feed, &sheet] {
        let lowered = page.to_lowercase();
        assert!(!lowered.contains("you have not"), "the pet addressed the reader");
        assert!(!lowered.contains("your archive"), "the pet addressed the reader");
    }
}

#[sqlx::test]
async fn the_familiar_page_stands_up_with_no_posts_at_all(pool: SqlitePool) {
    let (status, body) = get(&app(pool), "/familiar").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("still an egg"), "{body}");
    assert!(body.contains("waiting for a first post"), "{body}");
}
