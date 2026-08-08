//! The authoring API over `posts`: create, read, edit, delete, preview.

mod common;

use axum::http::StatusCode;
use common::{
    create_post, create_reply, empty_request, get, json_request, json_str, login, send, PASSWORD,
};
use sqlx::SqlitePool;

#[sqlx::test]
async fn every_post_route_requires_a_session(pool: SqlitePool) {
    let app = common::app(pool);

    let unauthenticated = [
        get("/api/posts", None),
        get("/api/posts/whatever", None),
        get("/api/drafts", None),
        get("/preview/whatever", None),
        json_request("POST", "/api/posts", r#"{"body":"hi"}"#, None),
        json_request("PATCH", "/api/posts/whatever", r#"{"body":"hi"}"#, None),
        empty_request("DELETE", "/api/posts/whatever", None),
    ];

    for request in unauthenticated {
        let uri = request.uri().to_string();
        let method = request.method().clone();
        let reply = send(&app, request).await;
        assert_eq!(
            reply.status,
            StatusCode::UNAUTHORIZED,
            "{method} {uri} is reachable without a session"
        );
    }
}

#[sqlx::test]
async fn creating_a_post_renders_it_and_never_leaks_the_rowid(pool: SqlitePool) {
    let app = common::app(pool);
    let cookie = login(&app).await;

    let reply = send(
        &app,
        json_request(
            "POST",
            "/api/posts",
            &format!(r#"{{"body":{}}}"#, json_str("hello *world*\nsecond line")),
            Some(&cookie),
        ),
    )
    .await;

    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let post = reply.json();

    assert_eq!(post["id"].as_str().expect("id").len(), 16);
    assert_eq!(post["visibility"], "public", "visibility defaults to public");
    assert_eq!(post["is_reply"], false);
    assert_eq!(post["reply_count"], 0);
    assert!(post["edited_at"].is_null(), "a new post is not edited");

    // The body round-trips as typed, and the rendered form went through the real
    // markdown pipeline.
    assert_eq!(post["body"], "hello *world*\nsecond line");
    let html = post["body_html"].as_str().unwrap();
    assert!(html.contains("<em>world</em>"), "{html}");
    assert!(html.contains("<br>"), "{html}");

    // The internal rowid is not part of the API and must not appear anywhere.
    assert!(post.get("rowid").is_none());
    assert!(
        post["id"].as_str().unwrap().parse::<i64>().is_err(),
        "the public id must not be the rowid"
    );
}

#[sqlx::test]
async fn a_post_needs_a_body_and_has_a_ceiling(pool: SqlitePool) {
    let app = common::app(pool);
    let cookie = login(&app).await;

    for empty in ["", "   ", "\n\n"] {
        let reply = send(
            &app,
            json_request(
                "POST",
                "/api/posts",
                &format!(r#"{{"body":{}}}"#, json_str(empty)),
                Some(&cookie),
            ),
        )
        .await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{empty:?}");
    }

    let too_long = "a".repeat(4001);
    let reply = send(
        &app,
        json_request(
            "POST",
            "/api/posts",
            &format!(r#"{{"body":{}}}"#, json_str(&too_long)),
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reply.json()["error"]["code"], "invalid");
}

#[sqlx::test]
async fn replies_chain_onto_the_parent_thread(pool: SqlitePool) {
    let app = common::app(pool);
    let cookie = login(&app).await;

    let root = create_post(&app, &cookie, "root", "public").await;
    let first = create_reply(&app, &cookie, &root, "first reply").await;
    // A reply to a reply stays on the same thread rather than starting one.
    let second = create_reply(&app, &cookie, &first, "second reply").await;

    let reply = send(&app, get(&format!("/api/posts/{second}"), Some(&cookie))).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);

    let payload = reply.json();
    let thread: Vec<&str> = payload["thread"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();

    assert_eq!(thread, vec![root.as_str(), first.as_str(), second.as_str()]);
    assert_eq!(payload["post"]["id"], second.as_str());
    assert_eq!(payload["post"]["is_reply"], true);
}

#[sqlx::test]
async fn replying_to_a_post_that_is_gone_is_rejected(pool: SqlitePool) {
    let app = common::app(pool);
    let cookie = login(&app).await;

    let reply = send(
        &app,
        json_request(
            "POST",
            "/api/posts",
            r#"{"body":"orphan","parent_id":"nonexistent00000"}"#,
            Some(&cookie),
        ),
    )
    .await;

    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", reply.body);
}

#[sqlx::test]
async fn the_authoring_feed_shows_every_visibility_unlike_the_public_one(pool: SqlitePool) {
    let app = common::app(pool.clone());
    let cookie = login(&app).await;

    create_post(&app, &cookie, "a public post", "public").await;
    create_post(&app, &cookie, "an unlisted post", "unlisted").await;
    let draft = create_post(&app, &cookie, "a draft", "draft").await;

    let reply = send(&app, get("/api/posts", Some(&cookie))).await;
    assert_eq!(reply.status, StatusCode::OK);

    let posts = reply.json()["posts"].as_array().unwrap().clone();
    assert_eq!(posts.len(), 3, "drafts and unlisted belong in the author's feed");

    // The public site, over the same data, shows only the public one.
    let (public_rows, _) = youwin_server::db::posts::feed_page(
        &pool,
        youwin_server::db::posts::Cursor::START,
        20,
    )
    .await
    .unwrap();
    assert_eq!(public_rows.len(), 1);

    let drafts = send(&app, get("/api/drafts", Some(&cookie))).await;
    let only = drafts.json()["posts"].as_array().unwrap().clone();
    assert_eq!(only.len(), 1);
    assert_eq!(only[0]["id"], draft.as_str());
}

#[sqlx::test]
async fn editing_a_published_post_marks_it_edited_and_re_renders(pool: SqlitePool) {
    let app = common::app(pool);
    let cookie = login(&app).await;
    let id = create_post(&app, &cookie, "before", "public").await;

    let reply = send(
        &app,
        json_request(
            "PATCH",
            &format!("/api/posts/{id}"),
            &format!(r#"{{"body":{}}}"#, json_str("after **bold**")),
            Some(&cookie),
        ),
    )
    .await;

    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let post = reply.json();

    assert_eq!(post["body"], "after **bold**");
    assert!(post["body_html"].as_str().unwrap().contains("<strong>bold</strong>"));
    assert!(!post["body_html"].as_str().unwrap().contains("before"));
    assert!(post["edited_at"].is_i64(), "a published edit is marked");
}

#[sqlx::test]
async fn editing_a_draft_does_not_mark_it_edited(pool: SqlitePool) {
    let app = common::app(pool);
    let cookie = login(&app).await;
    let id = create_post(&app, &cookie, "rough", "draft").await;

    // Re-saving something unpublished is not "editing" in the sense the marker
    // means — a draft revised ten times before publishing should read as new.
    let reply = send(
        &app,
        json_request(
            "PATCH",
            &format!("/api/posts/{id}"),
            &format!(r#"{{"body":{}}}"#, json_str("less rough")),
            Some(&cookie),
        ),
    )
    .await;

    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert!(reply.json()["edited_at"].is_null());
}

#[sqlx::test]
async fn publishing_a_draft_changes_visibility_without_marking_an_edit(pool: SqlitePool) {
    let app = common::app(pool);
    let cookie = login(&app).await;
    let id = create_post(&app, &cookie, "ready", "draft").await;

    let reply = send(
        &app,
        json_request(
            "PATCH",
            &format!("/api/posts/{id}"),
            r#"{"visibility":"public"}"#,
            Some(&cookie),
        ),
    )
    .await;

    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["visibility"], "public");
    assert!(
        reply.json()["edited_at"].is_null(),
        "flipping visibility is not a text edit"
    );
}

#[sqlx::test]
async fn a_patch_that_changes_nothing_is_rejected(pool: SqlitePool) {
    let app = common::app(pool);
    let cookie = login(&app).await;
    let id = create_post(&app, &cookie, "unchanged", "public").await;

    let reply = send(
        &app,
        json_request("PATCH", &format!("/api/posts/{id}"), "{}", Some(&cookie)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);

    // …and an unknown id is a 404, not a silent success.
    let missing = send(
        &app,
        json_request(
            "PATCH",
            "/api/posts/nonexistent00000",
            r#"{"body":"hi"}"#,
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn deleting_a_thread_root_takes_its_replies_with_it(pool: SqlitePool) {
    let app = common::app(pool);
    let cookie = login(&app).await;

    let root = create_post(&app, &cookie, "root", "public").await;
    let reply_id = create_reply(&app, &cookie, &root, "reply").await;

    let reply = send(
        &app,
        empty_request("DELETE", &format!("/api/posts/{root}"), Some(&cookie)),
    )
    .await;

    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    // Leaving the replies behind would strand them at their own permalinks with
    // the post they answered gone. The count is reported so the blast radius is
    // never a surprise.
    assert_eq!(reply.json()["deleted"], 2);

    for id in [&root, &reply_id] {
        let gone = send(&app, get(&format!("/api/posts/{id}"), Some(&cookie))).await;
        assert_eq!(gone.status, StatusCode::NOT_FOUND, "{id} survived");
    }
}

#[sqlx::test]
async fn deleting_a_reply_leaves_the_rest_of_the_thread(pool: SqlitePool) {
    let app = common::app(pool);
    let cookie = login(&app).await;

    let root = create_post(&app, &cookie, "root", "public").await;
    let first = create_reply(&app, &cookie, &root, "first").await;
    let second = create_reply(&app, &cookie, &root, "second").await;

    let reply = send(
        &app,
        empty_request("DELETE", &format!("/api/posts/{first}"), Some(&cookie)),
    )
    .await;
    assert_eq!(reply.json()["deleted"], 1);

    let thread = send(&app, get(&format!("/api/posts/{root}"), Some(&cookie))).await;
    let payload = thread.json();
    let ids: Vec<&str> = payload["thread"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec![root.as_str(), second.as_str()]);

    // Deleting something already gone is a 404, so a double-tap on a phone does
    // not report success twice.
    let again = send(
        &app,
        empty_request("DELETE", &format!("/api/posts/{first}"), Some(&cookie)),
    )
    .await;
    assert_eq!(again.status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn preview_renders_a_draft_through_the_public_templates(pool: SqlitePool) {
    let app = common::app(pool);
    let cookie = login(&app).await;
    let id = create_post(&app, &cookie, "a draft with **bold**", "draft").await;

    let reply = send(&app, get(&format!("/preview/{id}"), Some(&cookie))).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);

    let html = &reply.body;
    // The real published rendering: same layout, same stylesheet, same markup.
    assert!(html.contains("<strong>bold</strong>"), "{html}");
    assert!(html.contains(r#"class="post-body""#), "{html}");
    assert!(html.contains("/assets/test.css"), "{html}");

    // Canonical and og:url point at the PUBLIC origin — where the post will
    // live — not at the authoring host it is being served from.
    assert!(
        html.contains(&format!(r#"href="{}/p/{id}""#, common::PUBLIC_ORIGIN)),
        "{html}"
    );

    // An unpublished post must never be indexable, even from here.
    assert!(html.contains("noindex"), "{html}");
}

#[sqlx::test]
async fn preview_404s_for_an_unknown_id(pool: SqlitePool) {
    let app = common::app(pool);
    let cookie = login(&app).await;

    let reply = send(&app, get("/preview/nonexistent00000", Some(&cookie))).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn the_feed_paginates_and_the_cursor_is_opaque(pool: SqlitePool) {
    let app = common::app(pool);
    let cookie = login(&app).await;

    for n in 0..5 {
        create_post(&app, &cookie, &format!("post {n}"), "public").await;
    }

    let first = send(&app, get("/api/posts?limit=2", Some(&cookie))).await;
    let page = first.json();
    assert_eq!(page["posts"].as_array().unwrap().len(), 2);

    let cursor = page["next"].as_str().expect("more pages remain");
    assert!(
        cursor.parse::<i64>().is_err(),
        "the cursor must be opaque, not a raw id: {cursor}"
    );

    let second = send(
        &app,
        get(&format!("/api/posts?limit=2&cursor={cursor}"), Some(&cookie)),
    )
    .await;
    let next_page = second.json();
    assert_eq!(next_page["posts"].as_array().unwrap().len(), 2);

    // No overlap between the pages.
    let ids_one: Vec<&str> = page["posts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    let ids_two: Vec<&str> = next_page["posts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    assert!(ids_one.iter().all(|id| !ids_two.contains(id)), "{ids_one:?} {ids_two:?}");
}

#[sqlx::test]
async fn writes_are_refused_cross_origin(pool: SqlitePool) {
    let app = common::app(pool);
    let cookie = login(&app).await;

    let hostile = axum::http::Request::builder()
        .method("POST")
        .uri("/api/posts")
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .header(axum::http::header::ORIGIN, "https://evil.example")
        .header(axum::http::header::COOKIE, &cookie)
        .body(axum::body::Body::from(r#"{"body":"injected"}"#))
        .unwrap();

    let reply = send(&app, hostile).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);

    // Nothing was written.
    let feed = send(&app, get("/api/posts", Some(&cookie))).await;
    assert_eq!(feed.json()["posts"].as_array().unwrap().len(), 0);
}

/// Guards against the harness drifting from the real password.
#[sqlx::test]
async fn the_harness_password_is_the_one_the_app_expects(pool: SqlitePool) {
    let app = common::app(pool);
    assert_eq!(PASSWORD, "correct horse battery staple");
    let _ = login(&app).await;
}
