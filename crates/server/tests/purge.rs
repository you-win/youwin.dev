//! What the cache purger actually puts on the wire.
//!
//! The purge is fire-and-forget: it cannot fail a request, and a broken one
//! would show up as nothing worse than a stale page nobody notices. That makes
//! it exactly the kind of code that ships wrong and stays wrong, so these tests
//! stand a real HTTP server in front of it and read what arrives.
//!
//! This verifies the request shape — method, path, auth header, body — not that
//! Cloudflare accepts it. That last step needs a live zone and a real token.

mod common;

use std::sync::{Arc, Mutex};

use axum::{Json, Router, http::HeaderMap, routing::post};
use common::{create_post, json_request, login, send};
use serde_json::{Value, json};
use sqlx::SqlitePool;
use tokio::net::TcpListener;

/// One captured request.
#[derive(Debug, Clone)]
struct Captured {
    path: String,
    authorization: Option<String>,
    body: Value,
}

/// A stand-in for `api.cloudflare.com` on a loopback port.
///
/// Answers anything under `/zones/…/purge_cache` and records it. Returns the
/// success envelope Cloudflare returns, so the client takes its happy path.
async fn stub() -> (String, Arc<Mutex<Vec<Captured>>>) {
    let captured: Arc<Mutex<Vec<Captured>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = captured.clone();

    let app = Router::new().route(
        "/zones/{zone}/purge_cache",
        post(
            move |headers: HeaderMap, uri: axum::http::Uri, Json(body): Json<Value>| {
                let sink = sink.clone();
                async move {
                    sink.lock().unwrap().push(Captured {
                        path: uri.path().to_owned(),
                        authorization: headers
                            .get("authorization")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_owned),
                        body,
                    });
                    Json(json!({ "success": true, "errors": [], "messages": [] }))
                }
            },
        ),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    (base, captured)
}

/// The purge is spawned, so it lands after the handler has already replied.
/// Polls rather than sleeping a fixed interval so the test is neither flaky nor
/// slow.
async fn wait_for(captured: &Arc<Mutex<Vec<Captured>>>, count: usize) -> Vec<Captured> {
    for _ in 0..100 {
        {
            let seen = captured.lock().unwrap();
            if seen.len() >= count {
                return seen.clone();
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let seen = captured.lock().unwrap().clone();
    panic!("expected {count} purge requests, saw {}: {seen:?}", seen.len());
}

#[sqlx::test]
async fn a_write_purges_the_edge_cache(pool: SqlitePool) {
    let (base, captured) = stub().await;
    let app = common::app_purging_to(pool, &base);
    let cookie = login(&app).await;

    let id = create_post(&app, &cookie, "a post worth publishing", "public").await;
    let seen = wait_for(&captured, 1).await;

    assert_eq!(seen[0].path, "/zones/test-zone/purge_cache");
    assert_eq!(seen[0].authorization.as_deref(), Some("Bearer test-token"));
    // Everything, not a URL list — see `cache.rs` for why.
    assert_eq!(seen[0].body, json!({ "purge_everything": true }));

    // Editing and deleting purge too. An edit that stayed cached for five
    // minutes is the whole reason this exists.
    let edit = send(
        &app,
        json_request(
            "PATCH",
            &format!("/api/posts/{id}"),
            r#"{"body":"a post worth revising"}"#,
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(edit.status, axum::http::StatusCode::OK, "{}", edit.body);

    let removed = send(
        &app,
        common::empty_request("DELETE", &format!("/api/posts/{id}"), Some(&cookie)),
    )
    .await;
    assert_eq!(removed.status, axum::http::StatusCode::OK, "{}", removed.body);

    wait_for(&captured, 3).await;
}

#[sqlx::test]
async fn a_purge_that_fails_never_fails_the_write(pool: SqlitePool) {
    // Nothing is listening on this port. The post must still be created and
    // still return 201 — the write already committed, and turning a successful
    // post into an error because a CDN was unreachable would be the wrong trade.
    let app = common::app_purging_to(pool, "http://127.0.0.1:9");

    let cookie = login(&app).await;
    let id = create_post(&app, &cookie, "written while the CDN is down", "public").await;
    assert_eq!(id.len(), 16);

    // Give the spawned task time to fail, and confirm nothing panicked into the
    // runtime on the way.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let reply = send(&app, common::get("/api/posts", Some(&cookie))).await;
    assert_eq!(reply.status, axum::http::StatusCode::OK);
    assert_eq!(reply.json()["posts"].as_array().unwrap().len(), 1);
}

#[sqlx::test]
async fn purging_disabled_makes_no_requests_at_all(pool: SqlitePool) {
    let (base, captured) = stub().await;
    // The default harness has purging off, which is how the site runs until a
    // zone and token are configured.
    let app = common::app(pool);

    let cookie = login(&app).await;
    create_post(&app, &cookie, "quiet", "public").await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    assert!(captured.lock().unwrap().is_empty(), "stub at {base} was contacted");
}
